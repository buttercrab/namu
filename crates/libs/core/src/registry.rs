//! Workflows defined with `#[workflow]` register themselves here, so a binary that links them can
//! export their IR.

use std::collections::HashMap;

#[derive(Clone, Copy)]
pub struct WorkflowEntry {
    pub id: &'static str,
    pub build: fn() -> crate::ir::Workflow,
}

inventory::collect!(WorkflowEntry);

pub fn get_workflows() -> HashMap<String, WorkflowEntry> {
    inventory::iter::<WorkflowEntry>
        .into_iter()
        .map(|e| (e.id.to_string(), *e))
        .collect()
}
