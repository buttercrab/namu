use namu_core::ir::Workflow;
use namu_core::registry::{PackFn, TaskImpl, UnpackFn};

use crate::context::ContextManager;

pub mod simple_engine;

pub trait Engine<C: ContextManager> {
    type WorkflowId;
    type RunId;

    fn create_workflow(&self, workflow: Workflow) -> Self::WorkflowId;

    fn create_run(&self, workflow_id: Self::WorkflowId) -> Self::RunId;

    fn run(&self, run_id: Self::RunId);

    fn add_task(
        &self,
        task_name: &str,
        task: TaskImpl,
        pack: Option<PackFn>,
        unpack: Option<UnpackFn>,
    );
}
