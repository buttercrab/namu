//! The workflow IR shared by the compiler (macros, flow) and the engine.

pub mod builtin;
pub mod hoist;
pub mod inputs;
pub mod ir;
pub mod json;
pub mod liveness;
pub mod msgpack;
pub mod parallel;
pub mod registry;
pub mod scopes;
pub mod signature;
pub mod types;
pub mod value;

pub type ValueId = usize;
pub type OpId = usize;
