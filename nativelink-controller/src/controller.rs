use nativelink_config::cas_server::CasConfig;
use nativelink_config::schedulers::SchedulerConfig;
use nativelink_config::stores::StoreConfig;
use std::collections::HashMap;

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
pub struct NativeLinkStatus {
    /// Whether NativeLink is currently running
    pub running: bool,

    /// Last time NativeLink was started
    pub last_started: Option<String>,

    /// Any error message if NativeLink failed to start/run
    pub error: Option<String>,

    /// Port mappings for active services
    pub active_ports: HashMap<String, u16>,
}

#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "kube.rs",
    version = "v1alpha1",
    kind = "NativeLink",
    namespaced,
    status = "NativeLinkStatus",
    printcolumn = r#"{"name":"Running", "type":"boolean", "description":"Whether the process is running", "jsonPath":".status.running"}"#
)]
pub struct NativeLinkSpec {
    /// The NativeLink server configuration
    pub config: CasConfig,

    /// TODO(aaronmondal): Instead of these values, consider a "deployment"
    ///                    field that imports the K8s Deployment schema.

    /// Optional overrides for process management
    #[serde(default)]
    pub runtime: RuntimeConfig,

    /// The container image to use for the NativeLink Pod
    pub image: String,

    /// Number of replicas
    #[serde(default = "default_replicas")]
    pub replicas: i32,
}

fn default_replicas() -> i32 {
    1
}

// TODO(aaronmondal): Probably unnecessary to map these out. Consider importing
//                    from the k8s openapi schemas.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Default)]
pub struct RuntimeConfig {
    /// Arguments to pass to the NativeLink executable
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables to set for the process
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Working directory for the process
    pub working_dir: Option<String>,
}
