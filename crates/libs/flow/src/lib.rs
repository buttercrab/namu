mod builder;
mod graph;
mod ir;

pub use builder::{
    Builder, branch, call, call0, call1, call2, call3, call4, call5, call6, call7, call8, call9,
    jump, literal, phi, return_unit, return_value,
};
pub use graph::{Graph, TracedValue};
pub use ir::{BasicBlock, BlockId, Node, NodeKind, Terminator, Value};
pub use namu_macros::{task, workflow};
