pub mod prelude {
    pub use crate::task;
    pub use crate::workflow;
    pub use anyhow::Result;
}

pub mod __macro_exports {
    pub use anyhow::Result;
    pub use namu_core::{BatchedTask, SingleTask, StreamTask, Task, TaskContext};
    pub use namu_flow::{
        Builder, Graph, Node, NodeKind, Terminator, TracedValue, call, extract, literal, phi,
        seal_block_branch, seal_block_jump, seal_block_return_unit, seal_block_return_value,
    };
}

pub use namu_macros::task;
pub use namu_macros::workflow;
