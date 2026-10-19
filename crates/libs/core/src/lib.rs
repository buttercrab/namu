//! The workflow IR shared by the compiler (macros, flow) and the engine.

pub mod ir;
pub mod liveness;
pub mod registry;
pub mod scopes;
pub mod value;

pub type ValueId = usize;
pub type OpId = usize;
