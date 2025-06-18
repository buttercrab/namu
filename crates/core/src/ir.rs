use serde::{Deserialize, Serialize};

pub type ValueId = usize;
pub type OpId = usize;

#[derive(Debug, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub operations: Vec<Operation>,
}

// --- New SSA-friendly IR ----------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub enum OpKind {
    /// Compile-time literal constant – produces exactly one value
    Literal { value: String },

    /// Call to a user-defined task.  `inputs` are ValueIds; the call may
    /// produce *one or more* SSA results.
    Call { name: String, inputs: Vec<ValueId> },

    /// Static single-assignment phi node.
    Phi { from: Vec<(OpId, ValueId)> },

    /// Extract an element from a tuple value produced earlier.
    Extract { tuple: ValueId, index: usize },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Operation {
    /// Operation kind (opcode + attached data)
    pub kind: OpKind,

    /// SSA value ids produced by this operation (len ≥ 1)
    pub outputs: Vec<ValueId>,

    /// Control-flow successor
    pub next: Next,
}

// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
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
