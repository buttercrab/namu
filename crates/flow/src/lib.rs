pub mod builder;
pub mod ir;

pub use builder::{
    Builder, call, extract, literal, phi, seal_block_branch, seal_block_jump,
    seal_block_return_unit, seal_block_return_value,
};
pub use ir::{Graph, Node, NodeKind, Terminator, TracedValue, Value};
pub use namu_macros::{task, workflow};
