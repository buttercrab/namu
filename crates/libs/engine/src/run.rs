//! What every worker knows about a run.

use std::sync::Arc;

use namu_core::OpId;
use namu_core::ir::Workflow;
use serde_json::Value;

use crate::ids::{Minter, NodeId, RunId, ScopeId, WorkerId, owner};

/// A run's immutable description, installed on every worker that takes part in it.
#[derive(Debug)]
pub struct RunSpec {
    pub id: RunId,
    pub workflow: Workflow,
    /// For each op that fans out, the op that collects its scope.
    pub collect_ops: Vec<Option<OpId>>,
    /// The root context node and the root scope, both held by the run's owner.
    pub root: NodeId,
    pub root_scope: ScopeId,
}

impl RunSpec {
    /// Describes a new run owned by the worker minting its ids.
    pub fn new(workflow: Workflow, ids: &Minter) -> anyhow::Result<Arc<Self>> {
        let collect_ops = namu_core::scopes::collect_ops(&workflow)?;
        Ok(Arc::new(Self {
            id: RunId::new_v4(),
            workflow,
            collect_ops,
            root: ids.next(),
            root_scope: ids.next(),
        }))
    }

    /// The worker that owns the run: it holds the root scope and receives reports.
    pub fn owner(&self) -> WorkerId {
        owner(self.root_scope)
    }
}

/// What a finished run produced. Results arrive in no particular order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunOutcome {
    pub results: Vec<Value>,
    pub failures: Vec<String>,
}

impl RunOutcome {
    pub fn succeeded(&self) -> bool {
        self.failures.is_empty()
    }
}
