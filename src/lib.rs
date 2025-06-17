pub mod prelude {
    pub use crate::task;
    pub use crate::workflow;
    pub use anyhow::Result;
}

pub mod __macro_exports {
    pub use anyhow::Result;
    pub use namu_core::{BatchedTask, SingleTask, StreamTask, Task, TaskContext};
    pub use namu_graph::{
        Builder, Graph, Node, NodeKind, Terminator, TracedValue, new_literal, phi,
    };
}

pub use namu_macros::task;
pub use namu_macros::workflow;
