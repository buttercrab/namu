//! Messages between workers. They are the only traffic on the per-step path.

use std::collections::BTreeMap;
use std::sync::Arc;

use namu_core::OpId;
use namu_core::ir::Workflow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{NodeId, RunId, ScopeId, WorkerId};
use crate::kernel::Start;
use crate::run::{RunInfo, RunOutcome, WorkerStats};
use crate::scope::Weight;
use crate::tree::Node;

/// A branch's identity, stable across handoffs and recovery: the scope it was created in and its
/// item index there. The run's root branch is item 0 of the root scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BranchId {
    pub scope: ScopeId,
    pub index: u64,
}

/// One execution path through a workflow, at some point of its progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub id: BranchId,
    /// How many times the branch has moved; each stop on a worker is its own log record.
    pub hop: u64,
    pub ctx: NodeId,
    pub resume: Resume,
    /// This branch's share of its innermost scope.
    pub weight: Weight,
    /// The scopes it is in, innermost last. The first is the run's root scope.
    pub scopes: Arc<[ScopeId]>,
}

impl Branch {
    pub fn scope(&self) -> ScopeId {
        *self
            .scopes
            .last()
            .expect("every branch is in the root scope")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resume {
    /// Continue interpreting the IR.
    Kernel(Start),
    /// Run the task call at `op`; its context is already prepared.
    Call { op: OpId },
}

/// What a branch reports to its run's owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Report {
    /// The workflow returned this value.
    Result(Value),
    /// The branch failed and was dropped.
    Failure(String),
}

/// Marks a message its sender keeps until the receiver acknowledges it (design §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub from: WorkerId,
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    /// Start a run owned by the receiving worker; `Finished` goes to `reply_to` when it is over.
    Start {
        run: RunId,
        workflow: Workflow,
        task_versions: BTreeMap<String, String>,
        reply_to: WorkerId,
    },
    Finished {
        run: RunId,
        outcome: RunOutcome,
    },

    /// Continue `branch` on the receiving worker. `nodes` are the nodes the sender created for
    /// this branch since it arrived there, innermost first: the receiver cannot hold them yet,
    /// and is likely to read them.
    Handoff {
        run: RunId,
        branch: Branch,
        nodes: Vec<Arc<Node>>,
        delivery: Delivery,
    },
    /// `branch` ended in `scope` and returns its weight, with its item if it reached the collect.
    End {
        run: RunId,
        scope: ScopeId,
        branch: BranchId,
        weight: Weight,
        item: Option<Value>,
        delivery: Delivery,
    },
    /// A branch's report to the run's owner. It carries the branch's weight in `scope`, which the
    /// owner returns once the report is recorded, so the run cannot finish before its reports
    /// arrive.
    Report {
        run: RunId,
        scope: ScopeId,
        branch: BranchId,
        weight: Weight,
        report: Report,
        delivery: Delivery,
    },
    /// The receiver has made what these deliveries produced durable; the sender may let go.
    Ack {
        seqs: Vec<u64>,
    },

    /// Asks the node's owner for a node.
    FetchNode {
        run: RunId,
        id: NodeId,
        reply_to: WorkerId,
    },
    Node {
        run: RunId,
        id: NodeId,
        node: Option<Arc<Node>>,
    },
    /// Asks the run's owner how the run is described, on first contact with the run.
    FetchRun {
        run: RunId,
        reply_to: WorkerId,
    },
    Run {
        info: Arc<RunInfo>,
    },
    /// The answer to `FetchRun` for a run that is over: its late messages are dropped.
    RunOver {
        run: RunId,
    },

    /// The run is over: drop its state and tell the owner what you did for it.
    Forget {
        run: RunId,
        reply_to: WorkerId,
    },
    Forgotten {
        run: RunId,
        worker: WorkerId,
        stats: WorkerStats,
    },
}

impl Message {
    /// The delivery to acknowledge, for messages their sender keeps until acknowledged.
    pub fn delivery(&self) -> Option<Delivery> {
        match self {
            Message::Handoff { delivery, .. }
            | Message::End { delivery, .. }
            | Message::Report { delivery, .. } => Some(*delivery),
            _ => None,
        }
    }
}
