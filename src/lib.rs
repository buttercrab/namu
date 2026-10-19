//! NAMU: workflows compiled once to IR and run on any number of machines.

pub mod prelude {
    pub use anyhow::Result;

    pub use crate::{Data, task, workflow};
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
    pub use inventory;
    pub use namu_core::ir::Workflow;
    pub use namu_core::registry::WorkflowEntry;
    pub use namu_flow::{
        Builder, Graph, IntoArg, Node, NodeKind, TracedValue, branch, call, call0, call1, call2,
        call3, call4, call5, call6, call7, call8, call9, catch, jump, literal, phi, return_unit,
        return_value,
    };
}

pub use namu_core::value::{Data, Value};
pub use namu_macros::{Data, task, workflow};
