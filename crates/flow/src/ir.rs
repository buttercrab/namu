//! The Intermediate Representation (IR) of the computational graph.
//!
//! This module defines the core data structures that represent the graph,
//! such as `Graph`, `Node`, `BasicBlock`, and `Terminator`.

use std::any::Any;
use std::sync::Arc;

use namu_core::ValueId;

pub type Value = Arc<dyn Any + Send + Sync>;
pub type NodeId = usize;
pub type BlockId = usize;

#[derive(Clone)]
pub enum NodeKind {
    Call {
        task_id: String,
        inputs: Vec<ValueId>,
    },
    Literal {
        value: Value,
        debug_repr: String,
    },
    Phi {
        from: Vec<(BlockId, ValueId)>,
    },
}

impl NodeKind {
    pub fn call(task_id: String, inputs: Vec<ValueId>) -> Self {
        Self::Call { task_id, inputs }
    }

    pub fn literal(value: Value, debug_repr: String) -> Self {
        Self::Literal { value, debug_repr }
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
