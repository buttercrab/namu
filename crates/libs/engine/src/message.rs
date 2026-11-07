//! Messages between workers. They are the only traffic on the per-step path.

use std::collections::BTreeMap;
use std::sync::Arc;

use namu_core::ir::Workflow;
use namu_core::value::Value;
use namu_core::{OpId, ValueId};
use serde::{Deserialize, Serialize};

use crate::ids::{Lineage, NodeId, RunId, ScopeId, WorkerId};
use crate::kernel::Start;
use crate::routes::Load;
use crate::run::{RunInfo, WorkerStats};
use crate::tree::Node;

/// A branch's identity, stable across handoffs and recovery: the scope it was created in (its
/// innermost scope) and its index there. The run's root branch is branch 0 of the root scope.
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
    /// Its logical identity, which names its calls with its clock.
    pub lineage: Lineage,
    /// Its logical clock: how many task calls and fan-outs it has made. Logged with it, so a
    /// branch run again from a record counts as it did the first time (see
    /// [`CallKey`](crate::tasks::CallKey)).
    pub clock: u64,
    pub ctx: NodeId,
    pub resume: Resume,
    /// The scopes it is in, innermost (`id.scope`) last. The first is the run's root scope.
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
    /// Perform the action at `op` (a call, a fan-out); its context is already prepared.
    Action { op: OpId },
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
    /// Start a run owned by the receiving worker, with its inputs bound to their values. Its
    /// reports go to `reply_to` as they are recorded (`Recorded`), and `Finished` when it is over.
    Start {
        run: RunId,
        workflow: Workflow,
        inputs: Vec<(ValueId, Value)>,
        task_versions: BTreeMap<String, String>,
        reply_to: WorkerId,
    },
    /// A report the run's owner recorded, for whoever started the run, which keeps it: results
    /// arrive as the run goes, an unbounded run's too. `branch` names it, so one sent again (a
    /// successor sends what it finds in the log) is known as the same. The owner keeps it until
    /// told it is `Stored`; then its record leaves the log.
    Recorded {
        run: RunId,
        branch: BranchId,
        report: Report,
        delivery: Delivery,
    },
    /// The run's starter has stored these `Recorded` deliveries of the receiver's.
    Stored {
        seqs: Vec<u64>,
    },
    /// The run is over. Its reports went before, as `Recorded`; here is what each worker did.
    Finished {
        run: RunId,
        workers: BTreeMap<WorkerId, WorkerStats>,
    },

    Handoff(Handoff),
    End(End),
    Report(Reported),
    /// The receiver has made what these deliveries produced durable; the sender may let go. It
    /// carries the receiver's load (design §10.3).
    Ack {
        from: WorkerId,
        seqs: Vec<u64>,
        load: Load,
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
    FetchRun(FetchRun),
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
            Message::Handoff(Handoff { delivery, .. })
            | Message::End(End { delivery, .. })
            | Message::Report(Reported { delivery, .. }) => Some(*delivery),
            _ => None,
        }
    }
}

/// Continue `branch` on the receiving worker. `nodes` are the nodes the sender created for this
/// branch since it arrived there, innermost first: the receiver cannot hold them yet, and is
/// likely to read them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub run: RunId,
    pub branch: Branch,
    pub nodes: Vec<Arc<Node>>,
    pub delivery: Delivery,
}

/// `branch` ended, with its item if it reached the collect. Goes to the owner of its scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct End {
    pub run: RunId,
    pub branch: BranchId,
    pub item: Option<Value>,
    pub delivery: Delivery,
}

/// A branch's report to the run's owner. The owner ends the branch in its scope once the report
/// is recorded, so the run cannot finish before its reports arrive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reported {
    pub run: RunId,
    pub branch: BranchId,
    pub report: Report,
    pub delivery: Delivery,
}

/// Asks the run's owner how the run is described, on first contact with the run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchRun {
    pub run: RunId,
    pub reply_to: WorkerId,
}

macro_rules! message_from {
    ($($variant:ident($type:ident)),*) => {$(
        impl From<$type> for Message {
            fn from(message: $type) -> Self {
                Message::$variant(message)
            }
        }
    )*};
}

message_from!(
    Handoff(Handoff),
    End(End),
    Report(Reported),
    FetchRun(FetchRun)
);
