use common::Workflow;

use crate::context::ContextManager;

mod naive;

pub trait Engine<C: ContextManager> {
    type WorkflowId;
    type RunId;

    fn create_workflow(&self, workflow: Workflow) -> Self::WorkflowId;

    fn create_run(&self, workflow_id: Self::WorkflowId) -> Self::RunId;

    fn start_run(&self, run_id: Self::RunId);
}
