use serde::{Deserialize, Serialize};

use crate::{OpId, ValueId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workflow {
    pub name: String,
    pub operations: Vec<Operation>,
}

impl Workflow {
    pub fn new(name: String, operations: Vec<Operation>) -> Self {
        Self { name, operations }
    }
}

// --- New grouped IR representation -----------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Literal {
    pub output: ValueId,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Phi {
    pub output: ValueId,
    /// Pairs of (predecessor Operation id, ValueId coming from that op)
    pub from: Vec<(OpId, ValueId)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Call {
    pub task_id: String,
    pub inputs: Vec<ValueId>,
    pub outputs: Vec<ValueId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Operation {
    /// Zero or more literal constants produced *before* any phi or call.
    pub literals: Vec<Literal>,

    /// Zero or more phi nodes evaluated after literals and before call.
    pub phis: Vec<Phi>,

    /// Optional task invocation.  When `None`, this operation represents a
    /// basic-block that ends with only literals/phis.
    pub call: Option<Call>,

    /// Control-flow successor metadata.
    pub next: Next,
}

impl Operation {
    pub fn new(literals: Vec<Literal>, phis: Vec<Phi>, call: Option<Call>, next: Next) -> Self {
        Self {
            literals,
            phis,
            call,
            next,
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Next {
    Jump {
        next: OpId,
    },
    Branch {
        var: ValueId,
        true_next: OpId,
        false_next: OpId,
    },
    Return {
        var: Option<ValueId>,
    },
}

impl Next {
    pub fn jump(next: OpId) -> Self {
        Self::Jump { next }
    }

    pub fn branch(var: ValueId, true_next: OpId, false_next: OpId) -> Self {
        Self::Branch {
            var,
            true_next,
            false_next,
        }
    }

    pub fn return_value(var: ValueId) -> Self {
        Self::Return { var: Some(var) }
    }

    pub fn return_unit() -> Self {
        Self::Return { var: None }
    }
}
