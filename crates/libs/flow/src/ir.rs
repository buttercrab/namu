//! The Intermediate Representation (IR) of the computational graph.
//!
//! This module defines the core data structures that represent the graph,
//! such as `Graph`, `Node`, `BasicBlock`, and `Terminator`.

use namu_core::ValueId;
use namu_core::ir::{BuiltinOp, ScalarType};

pub type NodeId = usize;
pub type BlockId = usize;

#[derive(Clone)]
pub enum NodeKind {
    Call {
        task_id: String,
        inputs: Vec<ValueId>,
    },
    /// A constant, as JSON.
    Literal {
        json: String,
    },
    Phi {
        from: Vec<(BlockId, ValueId)>,
    },
    /// Fan out over the list `input`; the single output is bound to each element.
    FanOut {
        input: ValueId,
    },
    /// Collect `input` from every branch of the innermost fan-out scope into a list.
    Collect {
        input: ValueId,
    },
    /// An expression the engine evaluates itself.
    Builtin {
        op: BuiltinOp,
        ty: ScalarType,
        inputs: Vec<ValueId>,
    },
}

impl NodeKind {
    pub fn call(task_id: String, inputs: Vec<ValueId>) -> Self {
        Self::Call { task_id, inputs }
    }

    pub fn literal(json: String) -> Self {
        Self::Literal { json }
    }

    pub fn phi(from: Vec<(BlockId, ValueId)>) -> Self {
        Self::Phi { from }
    }
}

pub struct Node {
    pub kind: NodeKind,
    pub outputs: Vec<ValueId>,
}

impl Node {
    pub fn new(kind: NodeKind, outputs: Vec<ValueId>) -> Self {
        Self { kind, outputs }
    }
}

pub enum Terminator {
    Jump {
        target: BlockId,
    },
    Branch {
        condition: ValueId,
        true_target: BlockId,
        false_target: BlockId,
    },
    Return {
        value: Option<ValueId>,
    },
    /// Ends a block whose last instruction is a call: to `ok` if the task succeeds, else to `err`
    /// with its error message bound to `error`.
    Catch {
        ok: BlockId,
        err: BlockId,
        error: ValueId,
    },
}

impl Terminator {
    pub fn jump(target: BlockId) -> Self {
        Self::Jump { target }
    }

    pub fn branch(condition: ValueId, true_target: BlockId, false_target: BlockId) -> Self {
        Self::Branch {
            condition,
            true_target,
            false_target,
        }
    }

    pub fn return_value(value: ValueId) -> Self {
        Self::Return { value: Some(value) }
    }

    pub fn return_unit() -> Self {
        Self::Return { value: None }
    }
}

#[derive(Default)]
pub struct BasicBlock {
    pub instructions: Vec<NodeId>,
    pub terminator: Option<Terminator>,
}

impl BasicBlock {
    pub fn new(instructions: Vec<NodeId>, terminator: Option<Terminator>) -> Self {
        Self {
            instructions,
            terminator,
        }
    }
}
