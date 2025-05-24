// Copyright 2024 The NativeLink Authors. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use core::borrow::Borrow;
use core::cmp::Eq;
use core::fmt::Debug;
use core::future::Future;
use core::hash::Hash;
use core::ops::RangeBounds;
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

use async_lock::Mutex;
use lru::LruCache;
use nativelink_config::stores::EvictionPolicy;
use nativelink_metric::MetricsComponent;
use opentelemetry::{InstrumentationScope, Key, KeyValue, global, metrics};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::instant_wrapper::InstantWrapper;

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct SerializedLRU<K> {
    pub data: Vec<(K, i32)>,
    pub anchor_time: u64,
}

#[derive(Debug)]
struct EvictionItem<T: LenEntry + Debug> {
    seconds_since_anchor: i32,
    data: T,
}

pub trait LenEntry: 'static {
    /// Length of referenced data.
    fn len(&self) -> u64;

    /// Returns `true` if `self` has zero length.
    fn is_empty(&self) -> bool;

    /// This will be called when object is removed from map.
    /// Note: There may still be a reference to it held somewhere else, which
    /// is why it can't be mutable. This is a good place to mark the item
    /// to be deleted and then in the Drop call actually do the deleting.
    /// This will ensure nowhere else in the program still holds a reference
    /// to this object.
    /// You should not rely only on the Drop trait. Doing so might result in the
    /// program safely shutting down and calling the Drop method on each object,
    /// which if you are deleting items you may not want to do.
    /// It is undefined behavior to have `unref()` called more than once.
    /// During the execution of `unref()` no items can be added or removed to/from
    /// the `EvictionMap` globally (including inside `unref()`).
    #[inline]
    fn unref(&self) -> impl Future<Output = ()> + Send {
        core::future::ready(())
    }
}

impl<T: LenEntry + Send + Sync> LenEntry for Arc<T> {
    #[inline]
    fn len(&self) -> u64 {
        T::len(self.as_ref())
    }

    #[inline]
    fn is_empty(&self) -> bool {
        T::is_empty(self.as_ref())
    }

    #[inline]
    async fn unref(&self) {
        self.as_ref().unref().await;
    }
}

/// Key describing the kind of store that the EvictingMap is used in.
///
/// Use it to create the EvictingMap like so:
///
///
/// ```rust
/// EvictingMap::new(
///     eviction_config,
///     (now_fn)(),
///     &[KeyValue::new(STORE.clone(), "label")],
/// ),
/// ```
pub static STORE: Key = Key::from_static_str("store");

static EVICTING_MAP_METRICS: LazyLock<EvictingMapMetrics> = LazyLock::new(|| {
    let meter = global::meter_with_scope(
        InstrumentationScope::builder("nativelink.common.evicting_map").build(),
    );

    EvictingMapMetrics {
        evicted_bytes: meter
            .u64_counter("nativelink.common.evicting_map.evicted_bytes")
            .with_description("Number of bytes evicted from the store")
            .build(),
        evicted_items: meter
            .u64_counter("nativelink.common.evicting_map.evicted_items")
            .with_description("Number of items evicted from the store")
            .build(),
        replaced_bytes: meter
            .u64_counter("nativelink.common.evicting_map.replaced_bytes")
            .with_description("Number of bytes replaced in the store")
            .build(),
        replaced_items: meter
            .u64_counter("nativelink.common.evicting_map.replaced_items")
            .with_description("Number of items replaced in the store")
            .build(),
        inserted_bytes: meter
            .u64_counter("nativelink.common.evicting_map.inserted_bytes")
            .with_description("Number of bytes inserted into the store")
            .build(),
        inserted_items: meter
            .u64_counter("nativelink.common.evicting_map.inserted_items")
            .with_description("Number of items inserted into the store")
            .build(),
        read_bytes: meter
            .u64_counter("nativelink.common.evicting_map.read_bytes")
            .with_description("Number of bytes read from the store")
            .build(),
        sum_store_size: meter
            .i64_up_down_counter("nativelink.common.evicting_map.sum_store_size")
            .with_description("Total size of all items in the store")
            .build(),
        item_size_bytes: meter
            .u64_histogram("nativelink.common.evicting_map.item_size_bytes")
            .with_description("Distribution of item sizes")
            .with_unit("By")
            .build(),
        cache_requests: meter
            .u64_counter("nativelink.common.evicting_map.cache_requests")
            .with_description("Total number of cache get requests")
            .build(),
    }
});

#[derive(Debug)]
struct EvictingMapMetrics {
    evicted_bytes: metrics::Counter<u64>,
    evicted_items: metrics::Counter<u64>,
    replaced_bytes: metrics::Counter<u64>,
    replaced_items: metrics::Counter<u64>,
    inserted_bytes: metrics::Counter<u64>,
    inserted_items: metrics::Counter<u64>,
    read_bytes: metrics::Counter<u64>,
    sum_store_size: metrics::UpDownCounter<i64>,
    item_size_bytes: metrics::Histogram<u64>,
    cache_requests: metrics::Counter<u64>,
}

#[derive(Debug, MetricsComponent)]
struct State<K: Ord + Hash + Eq + Clone + Debug + Send, T: LenEntry + Debug + Send> {
    lru: LruCache<K, EvictionItem<T>>,
    btree: Option<BTreeSet<K>>,
    // Total size of all items in the store.
    sum_store_size: u64,
}

impl<K: Ord + Hash + Eq + Clone + Debug + Send + Sync, T: LenEntry + Debug + Sync + Send>
    State<K, T>
{
    /// Removes an item from the cache.
    async fn remove<Q>(
        &mut self,
        key: &Q,
        eviction_item: &EvictionItem<T>,
        replaced: bool,
        attributes: &[KeyValue],
    ) where
        K: Borrow<Q>,
        Q: Ord + Hash + Eq + Debug + Sync,
    {
        let metrics = &*EVICTING_MAP_METRICS;

        if let Some(btree) = &mut self.btree {
            btree.remove(key.borrow());
        }

        let item_size = eviction_item.data.len();
        self.sum_store_size -= eviction_item.data.len();
        metrics
            .sum_store_size
            .add(-(TryInto::<i64>::try_into(item_size).unwrap()), attributes);

        if replaced {
            metrics.replaced_items.add(1, attributes);
            metrics.replaced_bytes.add(item_size, attributes);
        } else {
            metrics.evicted_items.add(1, attributes);
            metrics.evicted_bytes.add(item_size, attributes);
            metrics.item_size_bytes.record(item_size, attributes);
        }
        // Note: See comment in `unref()` requiring global lock of insert/remove.
        eviction_item.data.unref().await;
    }

    /// Inserts a new item into the cache. If the key already exists, the old item is returned.
    async fn put(
        &mut self,
        key: K,
        eviction_item: EvictionItem<T>,
        attributes: &[KeyValue],
    ) -> Option<T> {
        // If we are maintaining a btree index, we need to update it.
        if let Some(btree) = &mut self.btree {
            btree.insert(key.clone());
        }
        if let Some(old_item) = self.lru.put(key.clone(), eviction_item) {
            self.remove(&key, &old_item, true, attributes).await;
            return Some(old_item.data);
        }
        None
    }
}

#[derive(Debug, MetricsComponent)]
pub struct EvictingMap<
    K: Ord + Hash + Eq + Clone + Debug + Send,
    T: LenEntry + Debug + Send,
    I: InstantWrapper,
> {
    #[metric]
    state: Mutex<State<K, T>>,
    anchor_time: I,
    /// Maximum size of the store in bytes.
    max_bytes: u64,
    /// Number of bytes to evict when the store is full.
    evict_bytes: u64,
    /// Maximum number of seconds to keep an item in the store.
    max_seconds: i32,
    // Maximum number of items to keep in the store.
    max_count: u64,
    /// Pre-allocated attributes for metrics.
    metric_attributes: Vec<KeyValue>,
}

impl<K, T, I> EvictingMap<K, T, I>
where
    K: Ord + Hash + Eq + Clone + Debug + Send + Sync,
    T: LenEntry + Debug + Clone + Send + Sync,
    I: InstantWrapper,
{
    pub fn new(config: &EvictionPolicy, anchor_time: I, attributes: &[KeyValue]) -> Self {
        let mut metric_attributes = vec![
            // TODO(aaronmondal): This is out of place. The proper way to handle
            //                    this seems to be removing the StoreManager and
            //                    constructing the store layout by directly
            //                    using the graph structure implied by the
            //                    configuration.
            //
            //                    In the meantime, the allocation here is not
            //                    the end of the world as it only happens once
            //                    for each construction.
            KeyValue::new("instance_name", "unknown"),
        ];
        metric_attributes.extend_from_slice(attributes);

        Self {
            // We use unbounded because if we use the bounded version we can't call the delete
            // function on the LenEntry properly.
            state: Mutex::new(State {
                lru: LruCache::unbounded(),
                btree: None,
                sum_store_size: 0,
            }),
            anchor_time,
            max_bytes: config.max_bytes as u64,
            evict_bytes: config.evict_bytes as u64,
            max_seconds: config.max_seconds as i32,
            max_count: config.max_count,
            metric_attributes,
        }
    }

    pub async fn enable_filtering(&self) {
        let mut state = self.state.lock().await;
        if state.btree.is_none() {
            Self::rebuild_btree_index(&mut state);
        }
    }

    fn rebuild_btree_index(state: &mut State<K, T>) {
        state.btree = Some(state.lru.iter().map(|(k, _)| k).cloned().collect());
    }

    /// Run the `handler` function on each key-value pair that matches the `prefix_range`
    /// and return the number of items that were processed.
    /// The `handler` function should return `true` to continue processing the next item
    /// or `false` to stop processing.
    pub async fn range<F, Q>(&self, prefix_range: impl RangeBounds<Q> + Send, mut handler: F) -> u64
    where
        F: FnMut(&K, &T) -> bool + Send,
        K: Borrow<Q> + Ord,
        Q: Ord + Hash + Eq + Debug + Sync,
    {
        let mut state = self.state.lock().await;
        let btree = if let Some(ref btree) = state.btree {
            btree
        } else {
            Self::rebuild_btree_index(&mut state);
            state.btree.as_ref().unwrap()
        };
        let mut continue_count = 0;
        for key in btree.range(prefix_range) {
            let value = &state.lru.peek(key.borrow()).unwrap().data;
            let should_continue = handler(key, value);
            if !should_continue {
                break;
            }
            continue_count += 1;
        }
        continue_count
    }

    /// Returns the number of key-value pairs that are currently in the the cache.
    /// Function is not for production code paths.
    pub async fn len_for_test(&self) -> usize {
        self.state.lock().await.lru.len()
    }

    fn should_evict(
        &self,
        lru_len: usize,
        peek_entry: &EvictionItem<T>,
        sum_store_size: u64,
        max_bytes: u64,
    ) -> bool {
        let is_over_size = max_bytes != 0 && sum_store_size >= max_bytes;

        let evict_older_than_seconds =
            (self.anchor_time.elapsed().as_secs() as i32) - self.max_seconds;
        let old_item_exists =
            self.max_seconds != 0 && peek_entry.seconds_since_anchor < evict_older_than_seconds;

        let is_over_count = self.max_count != 0 && (lru_len as u64) > self.max_count;

        is_over_size || old_item_exists || is_over_count
    }

    async fn evict_items(&self, state: &mut State<K, T>) {
        let Some((_, mut peek_entry)) = state.lru.peek_lru() else {
            return;
        };

        let max_bytes = if self.max_bytes != 0
            && self.evict_bytes != 0
            && self.should_evict(
                state.lru.len(),
                peek_entry,
                state.sum_store_size,
                self.max_bytes,
            ) {
            self.max_bytes.saturating_sub(self.evict_bytes)
        } else {
            self.max_bytes
        };

        while self.should_evict(state.lru.len(), peek_entry, state.sum_store_size, max_bytes) {
            let (key, eviction_item) = state
                .lru
                .pop_lru()
                .expect("Tried to peek() then pop() but failed");
            debug!(?key, "Evicting",);
            state
                .remove(&key, &eviction_item, false, &self.metric_attributes)
                .await;

            peek_entry = if let Some((_, entry)) = state.lru.peek_lru() {
                entry
            } else {
                return;
            };
        }
    }

    /// Return the size of a `key`, if not found `None` is returned.
    pub async fn size_for_key<Q>(&self, key: &Q) -> Option<u64>
    where
        K: Borrow<Q>,
        Q: Ord + Hash + Eq + Debug + Sync,
    {
        let mut results = [None];
        self.sizes_for_keys([key], &mut results[..], false).await;
        results[0]
    }

    /// Return the sizes of a collection of `keys`. Expects `results` collection
    /// to be provided for storing the resulting key sizes. Each index value in
    /// `keys` maps directly to the size value for the key in `results`.
    /// If no key is found in the internal map, `None` is filled in its place.
    /// If `peek` is set to `true`, the items are not promoted to the front of the
    /// LRU cache. Note: peek may still evict, but won't promote.
    pub async fn sizes_for_keys<It, Q, R>(&self, keys: It, results: &mut [Option<u64>], peek: bool)
    where
        It: IntoIterator<Item = R> + Send,
        // Note: It's not enough to have the inserts themselves be Send. The
        // returned iterator should be Send as well.
        <It as IntoIterator>::IntoIter: Send,
        // This may look strange, but what we are doing is saying:
        // * `K` must be able to borrow `Q`
        // * `R` (the input stream item type) must also be able to borrow `Q`
        // Note: That K and R do not need to be the same type, they just both need
        // to be able to borrow a `Q`.
        K: Borrow<Q>,
        R: Borrow<Q> + Send,
        Q: Ord + Hash + Eq + Debug + Sync,
    {
        let metrics = &*EVICTING_MAP_METRICS;
        let mut state = self.state.lock().await;

        let lru_len = state.lru.len();
        for (key, result) in keys.into_iter().zip(results.iter_mut()) {
            metrics.cache_requests.add(1, &self.metric_attributes);

            let maybe_entry = if peek {
                state.lru.peek_mut(key.borrow())
            } else {
                state.lru.get_mut(key.borrow())
            };
            match maybe_entry {
                Some(entry) => {
                    // Note: We need to check eviction because the item might be expired
                    // based on the current time. In such case, we remove the item while
                    // we are here.
                    if self.should_evict(lru_len, entry, 0, u64::MAX) {
                        *result = None;
                        if let Some((key, eviction_item)) = state.lru.pop_entry(key.borrow()) {
                            info!(?key, "Item expired, evicting");
                            state
                                .remove(key.borrow(), &eviction_item, false, &self.metric_attributes)
                                .await;
                        }
                    } else {
                        if !peek {
                            entry.seconds_since_anchor =
                                self.anchor_time.elapsed().as_secs() as i32;
                        }
                        let data_len = entry.data.len();
                        *result = Some(data_len);

                        metrics.read_bytes.add(data_len, &self.metric_attributes);
                        metrics
                            .item_size_bytes
                            .record(data_len, &self.metric_attributes);
                    }
                }
                None => *result = None,
            }
        }
    }

    pub async fn get<Q>(&self, key: &Q) -> Option<T>
    where
        K: Borrow<Q>,
        Q: Ord + Hash + Eq + Debug + Sync,
    {
        let metrics = &*EVICTING_MAP_METRICS;
        let mut state = self.state.lock().await;
        metrics.cache_requests.add(1, &self.metric_attributes);

        self.evict_items(&mut *state).await;

        let entry = state.lru.get_mut(key.borrow())?;

        entry.seconds_since_anchor = self.anchor_time.elapsed().as_secs() as i32;

        let data = entry.data.clone();
        let data_len = data.len();

        metrics.read_bytes.add(data_len, &self.metric_attributes);
        metrics
            .item_size_bytes
            .record(data_len, &self.metric_attributes);

        Some(data)
    }

    /// Returns the replaced item if any.
    pub async fn insert(&self, key: K, data: T) -> Option<T> {
        self.insert_with_time(key, data, self.anchor_time.elapsed().as_secs() as i32)
            .await
    }

    /// Returns the replaced item if any.
    pub async fn insert_with_time(&self, key: K, data: T, seconds_since_anchor: i32) -> Option<T> {
        let mut state = self.state.lock().await;
        let results = self
            .inner_insert_many(&mut state, [(key, data)], seconds_since_anchor)
            .await;
        results.into_iter().next()
    }

    /// Same as `insert()`, but optimized for multiple inserts.
    /// Returns the replaced items if any.
    pub async fn insert_many<It>(&self, inserts: It) -> Vec<T>
    where
        It: IntoIterator<Item = (K, T)> + Send,
        // Note: It's not enough to have the inserts themselves be Send. The
        // returned iterator should be Send as well.
        <It as IntoIterator>::IntoIter: Send,
    {
        let mut inserts = inserts.into_iter().peekable();
        // Shortcut for cases where there are no inserts, so we don't need to lock.
        if inserts.peek().is_none() {
            return Vec::new();
        }
        let state = &mut self.state.lock().await;
        self.inner_insert_many(state, inserts, self.anchor_time.elapsed().as_secs() as i32)
            .await
    }

    async fn inner_insert_many<It>(
        &self,
        state: &mut State<K, T>,
        inserts: It,
        seconds_since_anchor: i32,
    ) -> Vec<T>
    where
        It: IntoIterator<Item = (K, T)> + Send,
        // Note: It's not enough to have the inserts themselves be Send. The
        // returned iterator should be Send as well.
        <It as IntoIterator>::IntoIter: Send,
    {
        let metrics = &*EVICTING_MAP_METRICS;

        let mut replaced_items = Vec::new();
        for (key, data) in inserts {
            let new_item_size = data.len();
            let eviction_item = EvictionItem {
                seconds_since_anchor,
                data,
            };

            if let Some(old_item) = state.put(key, eviction_item, &self.metric_attributes).await {
                replaced_items.push(old_item);
            }
            state.sum_store_size += new_item_size;

            metrics
                .sum_store_size
                .add(new_item_size.try_into().unwrap(), &self.metric_attributes);
            metrics
                .inserted_bytes
                .add(new_item_size, &self.metric_attributes);
            metrics.inserted_items.add(1, &self.metric_attributes);
            metrics
                .item_size_bytes
                .record(new_item_size, &self.metric_attributes);

            self.evict_items(state).await;
        }
        replaced_items
    }

    pub async fn remove<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Ord + Hash + Eq + Debug + Sync,
    {
        let mut state = self.state.lock().await;
        self.inner_remove(&mut state, key).await
    }

    async fn inner_remove<Q>(&self, state: &mut State<K, T>, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Ord + Hash + Eq + Debug + Sync,
    {
        self.evict_items(state).await;
        if let Some(entry) = state.lru.pop(key.borrow()) {
            state.remove(key, &entry, false, &self.metric_attributes).await;
            return true;
        }
        false
    }

    /// Same as `remove()`, but allows for a conditional to be applied to the
    /// entry before removal in an atomic fashion.
    pub async fn remove_if<Q, F>(&self, key: &Q, cond: F) -> bool
    where
        K: Borrow<Q>,
        Q: Ord + Hash + Eq + Debug + Sync,
        F: FnOnce(&T) -> bool + Send,
    {
        let mut state = self.state.lock().await;
        if let Some(entry) = state.lru.get(key.borrow()) {
            if !cond(&entry.data) {
                return false;
            }
            return self.inner_remove(&mut state, key).await;
        }
        false
    }
}
