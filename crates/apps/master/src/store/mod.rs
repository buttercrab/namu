//! What the control plane keeps: the registry of task and workflow versions, the members of the
//! mesh, and runs.
//!
//! - Versions are immutable: one is stored once, and storing it again finds the first.
//! - A member lasts as long as its lease; a worker whose lease ran out is still among the `alive`
//!   until the sweep buries it and names its successor.
//! - A run is what was asked for (its [`RunSpec`], which never changes), who owns it (which changes
//!   when its owner dies), and how it ended (its [`RunEnd`], written once).
//!
//! [`RedisStore`] is the one that runs; [`MemoryStore`] is for tests, as the engine's
//! `MemoryLog` is.

#[cfg(test)]
mod memory;
mod redis;

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use namu_proto::{Member, RunStatus, TaskManifest, WorkflowUploadRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};
use uuid::Uuid;

#[cfg(test)]
pub use self::memory::MemoryStore;
pub use self::redis::RedisStore;

/// What storing an immutable version did.
#[derive(Debug, Clone, PartialEq)]
pub enum Inserted<T> {
    New,
    /// The version was stored already, as this.
    Existing(T),
}

/// A run as it was asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSpec {
    pub workflow_id: String,
    pub version: String,
    /// As they arrived, checked: a successor that starts the run again binds them again.
    pub inputs: Map<String, Json>,
    pub created_at_ms: u64,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEnd {
    pub status: RunStatus,
    pub results: Vec<Json>,
    pub failures: Vec<String>,
    pub finished_at_ms: u64,
    /// Task calls per worker, by name.
    pub calls_by_worker: BTreeMap<String, u64>,
    /// Value-tree nodes each worker fetched from another, by name.
    pub remote_node_fetches_by_worker: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunRecord {
    pub spec: RunSpec,
    pub owner: u64,
    /// `None` while it runs.
    pub end: Option<RunEnd>,
}

#[async_trait]
pub trait Store: Send + Sync + 'static {
    /// Stores task version `manifest`, unless it is stored already.
    async fn insert_task(&self, manifest: &TaskManifest) -> anyhow::Result<Inserted<TaskManifest>>;
    async fn task(&self, id: &str, version: &str) -> anyhow::Result<Option<TaskManifest>>;
    /// Stores `workflow`'s version, unless it is stored already.
    async fn insert_workflow(
        &self,
        workflow: &WorkflowUploadRequest,
    ) -> anyhow::Result<Inserted<WorkflowUploadRequest>>;
    async fn workflow(
        &self,
        id: &str,
        version: &str,
    ) -> anyhow::Result<Option<WorkflowUploadRequest>>;

    /// An id no worker had: 1, 2, .. (0 is the orchestrator's).
    async fn next_worker_id(&self) -> anyhow::Result<u64>;
    /// Makes `member` a member, alive, with a lease of `ttl`.
    async fn join(&self, member: &Member, ttl: Duration) -> anyhow::Result<()>;
    /// Renews member `id`'s lease; false if it is no longer a member.
    async fn renew(&self, id: u64, ttl: Duration) -> anyhow::Result<bool>;
    /// Replaces a member's record, renewing its lease; false if it is no longer a member.
    async fn update_member(&self, member: &Member, ttl: Duration) -> anyhow::Result<bool>;
    /// Ends member `id`'s lease now.
    async fn end_lease(&self, id: u64) -> anyhow::Result<()>;
    /// The members whose lease holds, by id.
    async fn members(&self) -> anyhow::Result<Vec<Member>>;
    /// Workers that joined and are not yet buried: those without a lease have died.
    async fn alive(&self) -> anyhow::Result<Vec<u64>>;
    /// Worker `dead` is gone, and `successor` takes over what it held.
    async fn bury(&self, dead: u64, successor: u64) -> anyhow::Result<()>;
    /// Every dead worker, with the worker that took over from it.
    async fn successors(&self) -> anyhow::Result<BTreeMap<u64, u64>>;
    /// The names workers joined with, kept after they leave, so run statistics stay readable.
    async fn worker_names(&self, ids: &[u64]) -> anyhow::Result<Vec<Option<String>>>;

    /// A counter, for choosing the owners of new runs in turn.
    async fn next_owner(&self) -> anyhow::Result<u64>;
    async fn create_run(&self, id: Uuid, spec: &RunSpec, owner: u64) -> anyhow::Result<()>;
    async fn run(&self, id: Uuid) -> anyhow::Result<Option<RunRecord>>;
    /// The runs that have not ended, with their owners.
    async fn running(&self) -> anyhow::Result<Vec<(Uuid, u64)>>;
    async fn set_owner(&self, id: Uuid, owner: u64) -> anyhow::Result<()>;
    /// Run `id` ended as `end`; it is no longer running.
    async fn end_run(&self, id: Uuid, end: &RunEnd) -> anyhow::Result<()>;
}
