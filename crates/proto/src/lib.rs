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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManifest {
    pub task_id: String,
    pub version: String,
    pub task_kind: TaskKind,
    pub resource_class: String,
    pub capabilities: Vec<String>,
    pub input_arity: usize,
    pub output_arity: usize,
    pub input_schema: JsonValue,
    pub output_schema: JsonValue,
    pub checksum: String,
    pub abi_version: String,
    pub build_toolchain: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowUploadRequest {
    pub id: String,
    pub version: String,
    pub ir: JsonValue,
    pub task_versions: std::collections::HashMap<String, String>,
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
    pub status: String,
    pub progress: Progress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStartRequest {
    pub op_id: usize,
    pub ctx_id: usize,
    pub lease_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCompleteRequest {
    pub op_id: usize,
    pub ctx_id: usize,
    pub success: bool,
    pub output_json: Option<JsonValue>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueMessage {
    pub run_id: Uuid,
    pub op_id: usize,
    pub ctx_id: usize,
    pub task_id: String,
    pub task_version: String,
    pub input_ids: Vec<usize>,
    pub lease_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_values: Option<Vec<JsonValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_hashes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_refs: Option<Vec<Option<ValueRef>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueRef {
    #[serde(rename = "ref")]
    pub ref_uri: String,
    pub hash: Option<String>,
    pub size: Option<u64>,
    pub codec: Option<String>,
}
