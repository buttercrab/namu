pub mod prelude {
    pub use anyhow::Result;

    pub use crate::{register_task, task, r#type, workflow};
}

pub mod export {
    use std::fs;
    use std::path::Path;

    use anyhow::Context;

    pub fn write_all<P: AsRef<Path>>(out_dir: P) -> anyhow::Result<()> {
        let out_dir = out_dir.as_ref();
        fs::create_dir_all(out_dir).context("create workflow export dir")?;

        for entry in namu_core::registry::get_workflows().values() {
            let workflow = (entry.build)();
            let json = serde_json::to_string_pretty(&workflow)?;
            let file_path = out_dir.join(format!("{}.workflow.ir.json", entry.id));
            fs::write(&file_path, json)?;
        }

        Ok(())
    }
}

#[doc(hidden)]
pub mod __macro_exports {
    pub use anyhow::Result;
    pub use inventory;
    pub use namu_core::registry::{
        DeserializeFn, PackFn, TaskEntry, TaskImpl, TypeEntry, UnpackFn, WorkflowEntry,
    };
    pub use namu_core::ir::Workflow;
    pub use namu_core::{BatchedTask, SingleTask, StreamTask, Task, TaskContext, Value};
    pub use namu_flow::{
        Builder, Graph, Node, NodeKind, Terminator, TracedValue, branch, call, call0, call1, call2,
        call3, call4, call5, call6, call7, call8, call9, jump, literal, phi, return_unit,
        return_value,
    };
}

pub use namu_macros::{register_task, task, r#type, workflow};
