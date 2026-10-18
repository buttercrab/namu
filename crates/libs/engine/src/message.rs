//! Messages between workers. They are the only traffic on the per-step path.

use std::sync::Arc;

use namu_core::OpId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{NodeId, RunId, ScopeId, WorkerId};
use crate::kernel::Start;
use crate::scope::Weight;
use crate::tree::Node;

/// One execution path through a workflow, at some point of its progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
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

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    /// Continue `branch` on the receiving worker. `node` is the branch's context node when the
    /// sender holds it, which saves the receiver a fetch.
    Handoff {
        run: RunId,
        branch: Branch,
        node: Option<Arc<Node>>,
    },
    /// A branch of `scope` ended and returns its weight, with its item if it reached the collect.
    End {
        run: RunId,
        scope: ScopeId,
        weight: Weight,
        item: Option<Value>,
    },
    /// A branch's report to the run's owner. It carries the branch's weight in `scope`, which the
    /// owner returns once the report is recorded, so the run cannot finish before its reports
    /// arrive.
    Report {
        run: RunId,
        scope: ScopeId,
        weight: Weight,
        report: Report,
    },
    /// Asks the node's owner for a node.
    Fetch {
        run: RunId,
        node: NodeId,
        request: u64,
        reply_to: WorkerId,
    },
    Fetched {
        request: u64,
        node: Option<Arc<Node>>,
    },
    /// The run is over; drop its state.
    Forget { run: RunId },
}
