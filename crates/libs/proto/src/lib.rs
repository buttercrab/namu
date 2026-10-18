//! Wire types shared by the CLI, the orchestrator (control plane) and workers (data plane).

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Single,
    Batch,
    Stream,
}

/// Kept for manifest compatibility only: all task code is trusted, so trust is not enforced.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskTrust {
    #[default]
    Trusted,
    Restricted,
    Untrusted,
}

/// How a task's artifact is executed. Each task picks its own runtime; workers advertise which
/// runtimes they support and only receive work they can run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TaskRuntime {
    /// A WASM component (`namu:task` WIT world). Platform-independent.
    Wasm,
    /// A native shared library exporting the NAMU C ABI. Platform-specific.
    Native,
}

impl TaskRuntime {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskRuntime::Wasm => "wasm",
            TaskRuntime::Native => "native",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "wasm" => Some(TaskRuntime::Wasm),
            "native" => Some(TaskRuntime::Native),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManifest {
    pub task_id: String,
    pub version: String,
    pub task_kind: TaskKind,
    pub runtime: TaskRuntime,
    pub input_arity: usize,
    pub output_arity: usize,
    /// Platform of a native artifact (`<os>-<arch>`, e.g. `macos-aarch64`). WASM tasks have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default)]
    pub trust: TaskTrust,
    /// Commit the task's output before anything downstream runs, so recovery never re-runs it.
    /// Use for tasks with external side effects.
    #[serde(default)]
    pub checkpoint: bool,
    #[serde(default)]
    pub requires_gpu: bool,
    #[serde(default = "default_resource_class")]
    pub resource_class: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub input_schema: JsonValue,
    #[serde(default)]
    pub output_schema: JsonValue,
    /// `sha256:<hex>` of the task binary; filled in by `namu build`.
    #[serde(default)]
    pub checksum: String,
    #[serde(default = "default_abi_version")]
    pub abi_version: String,
    #[serde(default)]
    pub build_toolchain: String,
    #[serde(default)]
    pub created_at: String,
}

impl TaskManifest {
    /// Placement key: which workers can run this task. WASM runs anywhere; native artifacts only
    /// on their own platform.
    pub fn runtime_key(&self) -> String {
        match self.runtime {
            TaskRuntime::Wasm => "wasm".to_string(),
            TaskRuntime::Native => {
                format!("native:{}", self.platform.as_deref().unwrap_or("unknown"))
            }
        }
    }
}

/// `<os>-<arch>` of the current process, e.g. `macos-aarch64`.
pub fn host_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn default_resource_class() -> String {
    "cpu".to_string()
}

fn default_abi_version() -> String {
    "2".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowUploadRequest {
    pub id: String,
    pub version: String,
    pub ir: JsonValue,
    pub task_versions: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCreateRequest {
    pub workflow_id: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCreateResponse {
    pub run_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStatusResponse {
    /// `running`, `succeeded`, or `partial_failed` (some branches failed).
    pub status: String,
    pub results: Vec<JsonValue>,
    pub failures: Vec<String>,
    pub elapsed_ms: Option<u64>,
    /// Task calls executed per worker.
    pub calls_by_worker: BTreeMap<String, u64>,
    /// Value-tree nodes each worker fetched because another worker produced them.
    pub remote_node_fetches_by_worker: BTreeMap<String, u64>,
}

/// A worker joining the cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRegisterRequest {
    /// A readable name, e.g. `worker-1`. Not necessarily unique.
    pub name: String,
    /// Where the worker listens for other workers (`host:port`).
    pub addr: String,
    /// Runtime keys this worker serves, e.g. `wasm`, `native:macos-aarch64`.
    pub runtimes: Vec<String>,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRegisterResponse {
    /// Unique per registration; the worker's identity in the mesh.
    pub worker_id: u64,
    pub members: Vec<Member>,
}

/// A participant in the mesh. Id 0 is the control plane (the orchestrator), which serves no
/// runtimes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Member {
    pub id: u64,
    pub name: String,
    pub addr: String,
    pub runtimes: Vec<String>,
    pub platform: String,
}

/// Registered workers renew their membership with a heartbeat, which returns the current members.
/// A worker whose heartbeats stop is no longer a member; if it comes back, it registers anew.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatResponse {
    pub members: Vec<Member>,
}

/// How often workers send heartbeats, and how long membership lasts without one.
pub const HEARTBEAT_INTERVAL_MS: u64 = 1_000;
pub const MEMBERSHIP_TTL_MS: u64 = 5_000;

/// Redis key layout of the control plane.
pub mod keys {
    use uuid::Uuid;

    pub const WORKER_SEQ: &str = "namu:worker_seq";
    /// Hash: worker id -> name, kept after the worker leaves so run stats stay readable.
    pub const WORKER_NAMES: &str = "namu:worker_names";
    /// Round-robin cursor for choosing the owner of a new run.
    pub const OWNER_CURSOR: &str = "namu:owner_cursor";

    /// A live member's record; expires unless renewed by heartbeats.
    pub fn member(id: u64) -> String {
        format!("namu:member:{id}")
    }

    pub const MEMBER_PATTERN: &str = "namu:member:*";

    pub fn task(task_id: &str, version: &str) -> String {
        format!("namu:task:{task_id}:{version}")
    }

    pub fn workflow(id: &str, version: &str) -> String {
        format!("namu:workflow:{id}:{version}")
    }

    /// Hash: status, created_at_ms, finished_at_ms.
    pub fn run_state(run: Uuid) -> String {
        format!("namu:run:{run}:state")
    }

    pub fn run_results(run: Uuid) -> String {
        format!("namu:run:{run}:results")
    }

    pub fn run_failures(run: Uuid) -> String {
        format!("namu:run:{run}:failures")
    }

    /// Hash: worker name -> task calls.
    pub fn run_calls(run: Uuid) -> String {
        format!("namu:run:{run}:calls")
    }

    /// Hash: worker name -> nodes fetched from other workers.
    pub fn run_remote_fetches(run: Uuid) -> String {
        format!("namu:run:{run}:remote_fetches")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_tasks_are_placed_by_platform() {
        let mut manifest: TaskManifest = serde_json::from_value(serde_json::json!({
            "task_id": "t", "version": "1", "task_kind": "single", "runtime": "native",
            "input_arity": 1, "output_arity": 1, "platform": "linux-x86_64"
        }))
        .unwrap();
        assert_eq!(manifest.runtime_key(), "native:linux-x86_64");
        manifest.runtime = TaskRuntime::Wasm;
        assert_eq!(manifest.runtime_key(), "wasm");
        // Defaults keep manifests short.
        assert_eq!(manifest.abi_version, "2");
        assert_eq!(manifest.trust, TaskTrust::Trusted);
    }
}
