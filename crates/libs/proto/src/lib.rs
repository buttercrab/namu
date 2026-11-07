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

/// The task ABI artifacts are built for: the native C ABI's version, and the `namu:task` WIT
/// world's (major) version. It must match `namu_guest::NATIVE_ABI_VERSION`.
pub const TASK_ABI_VERSION: u32 = 6;

/// How a task's artifact is executed. Each task picks its own runtime; workers advertise which
/// runtimes they support and only receive work they can run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TaskRuntime {
    /// A WASM component (`namu:task` WIT world). Platform-independent.
    Wasm,
    /// A native shared library exporting the NAMU C ABI. Platform-specific.
    Native,
    /// A Python module (the `namu` SDK's `@namu.task`), run by a pool of Python processes on
    /// the worker, in the worker's Python environment.
    Python,
}

impl TaskRuntime {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskRuntime::Wasm => "wasm",
            TaskRuntime::Native => "native",
            TaskRuntime::Python => "python",
        }
    }

    pub const ALL: [TaskRuntime; 3] = [TaskRuntime::Wasm, TaskRuntime::Native, TaskRuntime::Python];

    /// Whether a file named `name` is the code of a task of this runtime, in its artifact.
    pub fn is_artifact(&self, name: &str) -> bool {
        let extensions: &[&str] = match self {
            TaskRuntime::Wasm => &[".wasm"],
            TaskRuntime::Native => &[".so", ".dylib", ".dll"],
            TaskRuntime::Python => &[".py"],
        };
        extensions.iter().any(|extension| name.ends_with(extension))
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "wasm" => Some(TaskRuntime::Wasm),
            "native" => Some(TaskRuntime::Native),
            "python" => Some(TaskRuntime::Python),
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
    /// For a stream task: how many of its items may be in flight at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<usize>,
    /// For a batch task: items in a session at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<usize>,
    /// For a batch task: how long an idle session waits for a full batch; it runs at once if
    /// unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wait_ms: Option<u64>,
    /// For a batch task: sessions per worker (e.g. one per GPU).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_slots: Option<usize>,
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
            TaskRuntime::Python => "python".to_string(),
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
    "4".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowUploadRequest {
    pub id: String,
    pub version: String,
    pub ir: JsonValue,
    pub task_versions: HashMap<String, String>,
}

/// A task or workflow version, published. `unchanged` if it already was, identically.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Published {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub unchanged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCreateRequest {
    pub workflow_id: String,
    pub version: String,
    /// A value for each of the workflow's parameters, by name.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub inputs: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCreateResponse {
    pub run_id: Uuid,
}

/// Where a run is.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    /// It ended, but some branches failed.
    PartialFailed,
}

/// Where a run is. Its results and failures, which an unbounded run keeps adding to, are read in
/// pages ([`Page`]); here are how many there are so far.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStatusResponse {
    pub status: RunStatus,
    pub results: u64,
    pub failures: u64,
    pub elapsed_ms: Option<u64>,
    /// Task calls executed per worker.
    pub calls_by_worker: BTreeMap<String, u64>,
    /// Value-tree nodes each worker fetched because another worker produced them.
    pub remote_node_fetches_by_worker: BTreeMap<String, u64>,
}

/// Part of a run's results (or failures), in the order they arrived: `GET
/// /runs/{id}/results?after=N&limit=M` gives those from the `N`th on. `next` is where the next page
/// starts. A run that is still `Running` may add more after it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: u64,
    pub status: RunStatus,
}

/// Which part of the results a page holds.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub after: u64,
    #[serde(default)]
    pub limit: Option<u64>,
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
    pub membership: Membership,
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
    /// Draining (design §10.4): still a member, but given no new work.
    #[serde(default)]
    pub draining: bool,
}

/// The mesh as the control plane sees it. Registered workers renew their membership with a
/// heartbeat, which returns this. A worker whose heartbeats stop is no longer a member: its
/// successor takes over its work (design §7.4), and if it comes back, it registers anew.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Membership {
    pub members: Vec<Member>,
    /// Dead worker -> the worker that took over its work.
    pub successors: BTreeMap<u64, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkersResponse {
    pub workers: Vec<Member>,
}

/// What the API answers when a request fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// How often workers send heartbeats, and how long membership lasts without one.
pub const HEARTBEAT_INTERVAL_MS: u64 = 1_000;
pub const MEMBERSHIP_TTL_MS: u64 = 5_000;

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
        assert_eq!(manifest.abi_version, "4");
        assert_eq!(manifest.trust, TaskTrust::Trusted);
    }
}
