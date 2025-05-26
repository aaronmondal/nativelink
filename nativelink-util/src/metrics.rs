// Copyright 2025 The NativeLink Authors. All rights reserved.
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

use std::sync::LazyLock;

use opentelemetry::{InstrumentationScope, KeyValue, Value, global, metrics};

// Common values for tracking metrics.

/// Used as `Key` for a store type. Currently this is a bit free-form.
///
/// `Value` examples: `memory`, `existence_cache`, `filesystem`
pub const CACHE_TYPE: &str = "cache.type";

/// Used as `Key` for a `CacheOperationName`.
pub const CACHE_OPERATION: &str = "cache.operation.name";

/// Types of cache operations for metrics classification.
///
/// These categories help operators understand cache behavior patterns and tune
/// performance. Each operation type has different performance characteristics
/// and tuning implications.
#[derive(Debug, Clone, Copy)]
pub enum CacheOperationName {
    /// Reading data from cache (get, peek, size queries, etc.)
    ///
    /// **For Developers**: These are your cache lookups - when your application
    /// asks "do you have this data?" or "give me this data".
    ///
    /// **For Operators**: High read volume is normal, but low hit rates might
    /// indicate cache sizing issues or poor cache key strategies. Monitor
    /// read latency for performance bottlenecks.
    Read,

    /// Writing data to cache (insert, update, replace, etc.)
    ///
    /// **For Developers**: These happen when your application stores new data
    /// in the cache or updates existing entries.
    ///
    /// **For Operators**: Write patterns indicate cache churn. High write
    /// volume relative to reads might suggest cache policies need tuning.
    /// Monitor write latency as it can block application threads.
    Write,

    /// Explicit removal of cache entries (user/application initiated)
    ///
    /// **For Developers**: When your application explicitly deletes cache
    /// entries because they're no longer needed or have been invalidated.
    ///
    /// **For Operators**: Delete patterns show intentional cache invalidation.
    /// Spikes might indicate bulk cleanup operations or cache invalidation
    /// storms that could impact performance.
    Delete,

    /// Automatic cache maintenance (evictions, TTL expiry, size management)
    ///
    /// **For Developers**: This happens automatically when cache policies
    /// trigger - you don't directly cause these operations.
    ///
    /// **For Operators**: This is where cache tuning matters most. High
    /// eviction rates suggest undersized caches (increase
    /// `max_bytes`/`max_count` in config). Eviction spikes indicate memory
    /// pressure or suboptimal eviction policies (adjust `max_seconds`,
    /// `evict_bytes`).
    Evict,
}

impl From<CacheOperationName> for Value {
    fn from(op: CacheOperationName) -> Self {
        match op {
            CacheOperationName::Read => Self::from("read"),
            CacheOperationName::Write => Self::from("write"),
            CacheOperationName::Delete => Self::from("delete"),
            CacheOperationName::Evict => Self::from("evict"),
        }
    }
}

/// Used as `Key` for a `CacheOperationResult`.
pub const CACHE_RESULT: &str = "cache.operation.result";

/// Results of cache operations, with context-specific meanings.
///
/// The meaning of each result depends on the operation type. This design
/// allows operators to create targeted alerts and dashboards.
#[derive(Debug, Clone, Copy)]
pub enum CacheOperationResult {
    /// Successfully found valid data (Read operations only)
    ///
    /// **Cache Hit**: The data was in cache and is still valid. This is the
    /// best case scenario for performance.
    ///
    /// **Operator Alert Target**: Low hit rates indicate cache sizing or
    /// policy issues. Aim for >80% hit rates in most scenarios.
    Hit,

    /// Data not found in cache (Read operations only)
    ///
    /// **Cache Miss**: The data wasn't in cache, requiring a fallback to the
    /// underlying data source (disk, network, computation, etc.).
    ///
    /// **Operator Alert Target**: High miss rates suggest cache is too small,
    /// TTL too short, or poor cache key distribution.
    Miss,

    /// Data found but no longer valid (Read operations only)
    ///
    /// **Expired Entry**: The data was in cache but exceeded its TTL or other
    /// validity criteria. Functionally similar to a miss but indicates
    /// different tuning needs.
    ///
    /// **Operator Alert Target**: High expiry rates might indicate TTL
    /// policies are too aggressive (increase `max_seconds` in config).
    Expired,

    /// Operation completed successfully (Write/Delete/Evict operations)
    ///
    /// **Successful Operation**: The cache operation completed as intended.
    /// For writes: data stored. For deletes: entry removed. For evictions:
    /// space freed according to policy.
    ///
    /// **Operator Monitoring**: Track success rates to identify system health.
    /// Sudden drops in success rates indicate underlying issues.
    Success,

    /// Operation failed (any operation type)
    ///
    /// **Failed Operation**: Something went wrong - out of memory, I/O error,
    /// lock contention, etc. The specific error would be logged separately.
    ///
    /// **Operator Alert Target**: Any error rate >0.1% warrants investigation.
    /// Could indicate resource exhaustion, configuration issues, or bugs.
    Error,
}

impl From<CacheOperationResult> for Value {
    fn from(result: CacheOperationResult) -> Self {
        match result {
            CacheOperationResult::Hit => Self::from("hit"),
            CacheOperationResult::Miss => Self::from("miss"),
            CacheOperationResult::Expired => Self::from("expired"),
            CacheOperationResult::Success => Self::from("success"),
            CacheOperationResult::Error => Self::from("error"),
        }
    }
}

/// Pre-computed attribute combinations for zero-allocation cache metrics
#[derive(Debug)]
pub struct CacheMetricAttrs {
    // Read operations
    read_hit: Vec<KeyValue>,
    read_miss: Vec<KeyValue>,
    read_expired: Vec<KeyValue>,

    // Write operations
    write_success: Vec<KeyValue>,
    write_error: Vec<KeyValue>,

    // Delete operations
    delete_success: Vec<KeyValue>,
    delete_miss: Vec<KeyValue>,
    delete_error: Vec<KeyValue>,

    // Evict operations
    evict_success: Vec<KeyValue>,
    evict_expired: Vec<KeyValue>,
}

impl CacheMetricAttrs {
    pub fn new(base_attrs: &[KeyValue]) -> Self {
        // Helper to create attribute vec for operation + result
        let make_attrs = |op: CacheOperationName, result: CacheOperationResult| {
            let mut attrs = base_attrs.to_vec();
            attrs.push(KeyValue::new(CACHE_OPERATION, op));
            attrs.push(KeyValue::new(CACHE_RESULT, result));
            attrs
        };

        Self {
            read_hit: make_attrs(CacheOperationName::Read, CacheOperationResult::Hit),
            read_miss: make_attrs(CacheOperationName::Read, CacheOperationResult::Miss),
            read_expired: make_attrs(CacheOperationName::Read, CacheOperationResult::Expired),

            write_success: make_attrs(CacheOperationName::Write, CacheOperationResult::Success),
            write_error: make_attrs(CacheOperationName::Write, CacheOperationResult::Error),

            delete_success: make_attrs(CacheOperationName::Delete, CacheOperationResult::Success),
            delete_miss: make_attrs(CacheOperationName::Delete, CacheOperationResult::Miss),
            delete_error: make_attrs(CacheOperationName::Delete, CacheOperationResult::Error),

            evict_success: make_attrs(CacheOperationName::Evict, CacheOperationResult::Success),
            evict_expired: make_attrs(CacheOperationName::Evict, CacheOperationResult::Expired),
        }
    }

    // Convenience getters
    pub fn read_hit(&self) -> &[KeyValue] {
        &self.read_hit
    }
    pub fn read_miss(&self) -> &[KeyValue] {
        &self.read_miss
    }
    pub fn read_expired(&self) -> &[KeyValue] {
        &self.read_expired
    }
    pub fn write_success(&self) -> &[KeyValue] {
        &self.write_success
    }
    pub fn write_error(&self) -> &[KeyValue] {
        &self.write_error
    }
    pub fn delete_success(&self) -> &[KeyValue] {
        &self.delete_success
    }
    pub fn delete_miss(&self) -> &[KeyValue] {
        &self.delete_miss
    }
    pub fn delete_error(&self) -> &[KeyValue] {
        &self.delete_error
    }
    pub fn evict_success(&self) -> &[KeyValue] {
        &self.evict_success
    }
    pub fn evict_expired(&self) -> &[KeyValue] {
        &self.evict_expired
    }
}

pub static CACHE_METRICS: LazyLock<CacheMetrics> = LazyLock::new(|| {
    let meter = global::meter_with_scope(InstrumentationScope::builder("nativelink").build());

    CacheMetrics {
        cache_operation_duration: meter
            .f64_histogram("cache.operation.duration")
            .with_description("Duration of cache operations in milliseconds")
            .with_unit("ms")
            // The range of these is quite large as a cache might be backed by
            // memory, a filesystem, or network storage.
            .with_boundaries(vec![
                0.0,    // 0ms
                0.001,  // 1 μs
                0.002,  // 2 μs
                0.005,  // 5 μs
                0.010,  // 10 μs
                0.020,  // 20 μs
                0.050,  // 50 μs
                0.100,  // 100 μs
                0.200,  // 200 μs
                0.500,  // 500 μs
                1.0,    // 1 ms
                2.0,    // 2 ms
                5.0,    // 5 ms
                10.0,   // 10 ms
                25.0,   // 25 ms
                50.0,   // 50 ms
                100.0,  // 100 ms
                250.0,  // 250 ms
                500.0,  // 500 ms
                1000.0, // 1 second
                2500.0, // 2.5 seconds
                5000.0, // 5 seconds
            ])
            .build(),

        cache_operations: meter
            .u64_counter("cache.operations")
            .with_description("Total cache operations by type and result")
            .build(),

        cache_io: meter
            .u64_counter("cache.bytes.transferred")
            .with_description("Total bytes processed by cache operations")
            .with_unit("By")
            .build(),

        cache_size: meter
            .i64_up_down_counter("cache.size")
            .with_description("Current total size of cached items")
            .with_unit("By")
            .build(),

        cache_entries: meter
            .i64_up_down_counter("cache.entries")
            .with_description("Current number of cached items")
            .with_unit("{entry}")
            .build(),

        cache_entry_size: meter
            .u64_histogram("cache.item.size")
            .with_description("Distribution of cached item sizes")
            .with_unit("By")
            .build(),
    }
});

#[derive(Debug)]
pub struct CacheMetrics {
    pub cache_operation_duration: metrics::Histogram<f64>,
    pub cache_operations: metrics::Counter<u64>,
    pub cache_io: metrics::Counter<u64>,
    pub cache_size: metrics::UpDownCounter<i64>,
    pub cache_entries: metrics::UpDownCounter<i64>,
    pub cache_entry_size: metrics::Histogram<u64>,
}
