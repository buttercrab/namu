pub mod builder;
pub mod ir;

pub use builder::{Builder, literal, phi};
pub use ir::{Graph, Node, NodeKind, Terminator, TracedValue, Value};
pub use namu_macros::{task, workflow};
