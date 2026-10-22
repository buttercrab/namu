//! The workflow IR shared by the compiler (macros, flow) and the engine.

pub mod builtin;
pub mod ir;
pub mod liveness;
pub mod parallel;
pub mod registry;
pub mod scopes;
pub mod types;
pub mod value;

pub type ValueId = usize;
pub type OpId = usize;
