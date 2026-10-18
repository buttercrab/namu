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

/// Everything a worker needs to execute a run, written once by the orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMeta {
    pub workflow_id: String,
    pub version: String,
    pub ir: JsonValue,
    pub task_versions: HashMap<String, String>,
    pub created_at_ms: u64,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRegisterRequest {
    pub worker_id: String,
    /// Runtime keys this worker serves, e.g. `wasm`, `native:macos-aarch64`.
    pub runtimes: Vec<String>,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRegisterResponse {
    /// Unique per registration; workers use it to mint globally unique context ids.
    pub worker_index: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub worker_id: String,
    pub worker_index: u64,
    pub runtimes: Vec<String>,
    pub platform: String,
}

/// A unit of work handed between workers. It references a value-tree node; values themselves
/// are read through the tree, never copied into the message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkItem {
    /// Continue driving `ctx` from `op` (arrived from `pred`).
    Continue {
        run_id: Uuid,
        ctx: u64,
        op: usize,
        pred: Option<usize>,
    },
    /// Execute the call at `op` in `ctx` (already prepared), then continue.
    Call { run_id: Uuid, ctx: u64, op: usize },
}

impl WorkItem {
    pub fn run_id(&self) -> Uuid {
        match self {
            WorkItem::Continue { run_id, .. } | WorkItem::Call { run_id, .. } => *run_id,
        }
    }
}

/// An immutable value-tree node as stored in Redis. Once written it never changes, including
/// its parent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRecord {
    pub parent: Option<u64>,
    /// Enclosing fan-out scopes, innermost last. Each entry is the id of the context that fanned
    /// out.
    pub scopes: Vec<u64>,
    pub values: Vec<(usize, JsonValue)>,
}

/// Redis key layout shared by the orchestrator and workers.
pub mod keys {
    use uuid::Uuid;

    pub const WORKER_SEQ: &str = "namu:worker_seq";
    pub const WORKERS: &str = "namu:workers";
    /// Work any worker can pick up (fan-out branches, continuations).
    pub const QUEUE_ANY: &str = "namu:q:any";
    pub const CONSUMER_GROUP: &str = "namu:workers";

    /// Work that needs a specific runtime key (see `TaskManifest::runtime_key`).
    pub fn queue_runtime(runtime_key: &str) -> String {
        format!("namu:q:rt:{runtime_key}")
    }

    pub fn task(task_id: &str, version: &str) -> String {
        format!("namu:task:{task_id}:{version}")
    }

    pub fn workflow(id: &str, version: &str) -> String {
        format!("namu:workflow:{id}:{version}")
    }

    pub fn run_meta(run: Uuid) -> String {
        format!("namu:run:{run}:meta")
    }

    /// Hash: status, live, failures_count, finished_at_ms.
    pub fn run_state(run: Uuid) -> String {
        format!("namu:run:{run}:state")
    }

    pub fn run_results(run: Uuid) -> String {
        format!("namu:run:{run}:results")
    }

    pub fn run_failures(run: Uuid) -> String {
        format!("namu:run:{run}:failures")
    }

    pub fn run_calls(run: Uuid) -> String {
        format!("namu:run:{run}:calls")
    }

    pub fn run_remote_fetches(run: Uuid) -> String {
        format!("namu:run:{run}:remote_fetches")
    }

    pub fn node(run: Uuid, ctx: u64) -> String {
        format!("namu:run:{run}:node:{ctx}")
    }

    /// Hash for a fan-out scope: `live` (open branches) and `collect_op`.
    pub fn scope(run: Uuid, scope_ctx: u64) -> String {
        format!("namu:run:{run}:scope:{scope_ctx}")
    }

    /// List of values collected for a fan-out scope.
    pub fn scope_items(run: Uuid, scope_ctx: u64) -> String {
        format!("namu:run:{run}:scope:{scope_ctx}:items")
    }
}

/// Context ids are globally unique without coordination: the high bits hold the registering
/// worker's index, the low bits a per-worker counter. Index 0 is the orchestrator (root nodes).
pub const CONTEXT_COUNTER_BITS: u32 = 40;

pub fn context_id(worker_index: u64, counter: u64) -> u64 {
    (worker_index << CONTEXT_COUNTER_BITS) | (counter & ((1 << CONTEXT_COUNTER_BITS) - 1))
}

/// Context id of a run's root node (minted by the orchestrator).
pub const ROOT_CONTEXT: u64 = 0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_ids_are_unique_per_worker() {
        assert_eq!(context_id(0, 0), ROOT_CONTEXT);
        assert_ne!(context_id(1, 5), context_id(2, 5));
        assert_eq!(context_id(3, 7) >> CONTEXT_COUNTER_BITS, 3);
    }

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

    #[test]
    fn work_items_round_trip() {
        let item = WorkItem::Call {
            run_id: Uuid::nil(),
            ctx: 42,
            op: 7,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains(r#""kind":"call""#));
        let back: WorkItem = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, WorkItem::Call { ctx: 42, op: 7, .. }));
    }
}
