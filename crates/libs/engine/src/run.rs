//! What every worker knows about a run.

use std::collections::BTreeMap;
use std::sync::Arc;

use namu_core::OpId;
use namu_core::ir::Workflow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{NodeId, RunId, ScopeId, WorkerId, owner};
use crate::tasks::TaskRef;

/// A run as it is described to the workers that take part in it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInfo {
    pub id: RunId,
    pub workflow: Workflow,
    /// The version of each task the workflow calls. Empty for in-process tasks.
    pub task_versions: BTreeMap<String, String>,
    /// The root context node and the root scope, both held by the run's owner.
    pub root: NodeId,
    pub root_scope: ScopeId,
    /// Who is told the outcome (the control plane), if anyone.
    pub reply_to: Option<WorkerId>,
}

impl RunInfo {
    /// The worker that owns the run: it holds the root scope and receives reports.
    pub fn owner(&self) -> WorkerId {
        owner(self.root_scope)
    }
}

/// A run's immutable description with what is derived from it once per worker.
#[derive(Debug)]
pub struct RunSpec {
    pub info: Arc<RunInfo>,
    /// For each op that fans out, the op that collects its scope.
    pub collect_ops: Vec<Option<OpId>>,
    /// For each op that calls a task, the fan-out that is the only use of its output.
    pub fused: Vec<Option<OpId>>,
    /// For each op that calls a task, the task at the run's version.
    tasks: Vec<Option<TaskRef>>,
}

impl RunSpec {
    pub fn from_info(info: Arc<RunInfo>) -> anyhow::Result<Self> {
        let collect_ops = namu_core::scopes::collect_ops(&info.workflow)?;
        let fused = namu_core::scopes::fused_fan_outs(&info.workflow);
        let tasks = info
            .workflow
            .operations
            .iter()
            .map(|op| {
                op.call.as_ref().map(|call| TaskRef {
                    version: info
                        .task_versions
                        .get(&call.task_id)
                        .cloned()
                        .unwrap_or_default(),
                    id: call.task_id.clone(),
                })
            })
            .collect();
        Ok(Self {
            info,
            collect_ops,
            fused,
            tasks,
        })
    }

    pub fn id(&self) -> RunId {
        self.info.id
    }

    pub fn workflow(&self) -> &Workflow {
        &self.info.workflow
    }

    /// The task called at `op`, if it calls one.
    pub fn task(&self, op: OpId) -> Option<&TaskRef> {
        self.tasks.get(op)?.as_ref()
    }

    /// Every task the run calls.
    pub fn tasks(&self) -> impl Iterator<Item = &TaskRef> {
        self.tasks.iter().flatten()
    }

    pub fn owner(&self) -> WorkerId {
        self.info.owner()
    }
}

/// What a finished run produced. Results arrive in no particular order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub results: Vec<Value>,
    pub failures: Vec<String>,
    /// What each worker that took part did.
    pub workers: BTreeMap<WorkerId, WorkerStats>,
}

impl RunOutcome {
    pub fn succeeded(&self) -> bool {
        self.failures.is_empty()
    }
}

/// One worker's share of a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerStats {
    /// Task calls it ran.
    pub calls: u64,
    /// Nodes it fetched from other workers.
    pub fetches: u64,
}
