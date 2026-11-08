//! NAMU: workflows compiled once to IR and run on any number of machines.

pub mod prelude {
    pub use anyhow::Result;
    /// One branch per item of a list: `let page = each(pages);` (in a workflow).
    pub use namu_flow::each;

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

/// That a `match`'s value is its last arm's variant, as `#[workflow]` checks it (a macro, so the
/// workflow macro leaves what is inside as it is).
#[doc(hidden)]
#[macro_export]
macro_rules! __expect_variant {
    ($builder:ident, $value:ident, $variant:literal) => {
        $crate::__macro_exports::expect_variant(&$builder, &$value, $variant)
    };
}

/// What a `match`'s option holds, as `#[workflow]` binds it in a `Some(x)` arm (a macro, so the
/// workflow macro leaves what is inside as it is).
#[doc(hidden)]
#[macro_export]
macro_rules! __unwrap_some {
    ($builder:ident, $value:ident) => {
        $crate::__macro_exports::unwrap_some(&$builder, &$value)
    };
}

/// A field of a `match` arm's variant, as `#[workflow]` binds it (a macro, so the workflow macro
/// leaves what is inside as it is).
#[doc(hidden)]
#[macro_export]
macro_rules! __variant_part {
    ($builder:ident, $value:ident, $variant:literal, $field:literal, $part:expr) => {
        $crate::__macro_exports::variant_part(&$builder, &$value, $variant, $field, $part)
    };
}

#[doc(hidden)]
pub mod __macro_exports {
    pub use inventory;
    pub use namu_core::ir::{ScalarOp, Workflow};
    pub use namu_core::registry::WorkflowEntry;
    pub use namu_core::types::{Def, Defs, Describe, Type, named};
    pub use namu_flow::{
        Builder, Graph, IntoArg, Node, NodeKind, Raises, TracedValue, arith, branch, call, call0,
        call1, call2, call3, call4, call5, call6, call7, call8, call9, catch, check_arms, compare,
        expect_variant, field, format, format_arg, index, input, is_some, is_variant, jump,
        literal, logic, negate, not, phi, plus, reduce, return_unit, return_value, tuple,
        typed_like, unwrap_some, variant_part, window,
    };
}

pub use namu_core::inputs::Inputs;
pub use namu_core::types::Describe;
pub use namu_core::value::{Data, Value};
pub use namu_macros::{Data, Describe, task, workflow};
