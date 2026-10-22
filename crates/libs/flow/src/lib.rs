mod builder;
mod graph;
mod ir;

pub use builder::{
    Builder, IntoArg, Number, Plus, Scalar, Signed, arith, branch, call, call0, call1, call2,
    call3, call4, call5, call6, call7, call8, call9, catch, compare, each, jump, literal, logic,
    negate, not, phi, plus, reduce, return_unit, return_value, typed_like, window,
};
pub use graph::{Graph, TracedValue};
pub use ir::{BasicBlock, BlockId, Node, NodeKind, Terminator};
pub use namu_macros::{task, workflow};
