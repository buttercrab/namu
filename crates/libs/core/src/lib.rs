//! The workflow IR shared by the compiler (macros, flow) and the engine.

pub mod ir;
pub mod registry;
pub mod scopes;

pub type ValueId = usize;
pub type OpId = usize;
