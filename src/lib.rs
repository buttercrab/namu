pub mod prelude {
    pub use crate::task;
    pub use crate::workflow;
    pub use anyhow::Result;
}

pub mod __macro_exports {
    pub use anyhow::Result;
    pub use namu_core::{BatchedTask, SingleTask, StreamTask, Task, TaskContext};
    pub use namu_flow::{
        Builder, Graph, Node, NodeKind, Terminator, TracedValue, branch, call, call0, call1, call2,
        call3, call4, call5, call6, call7, call8, call9, jump, literal, phi, return_unit,
        return_value,
    };
}

pub use namu_macros::task;
pub use namu_macros::workflow;
