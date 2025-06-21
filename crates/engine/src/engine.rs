use namu_core::ir::Workflow;
use namu_core::{DynamicTaskContext, Task, Value};

use crate::context::ContextManager;

pub mod simple_engine;

pub type PackFn = fn(Vec<Value>) -> Value;
pub type UnpackFn = fn(Value) -> Vec<Value>;
pub type TaskImpl<C> = Box<
    dyn Task<<C as ContextManager>::ContextId, DynamicTaskContext<<C as ContextManager>::ContextId>>
        + Send
        + Sync,
>;

pub trait Engine<C: ContextManager> {
    type WorkflowId;
    type RunId;

    fn create_workflow(&self, workflow: Workflow) -> Self::WorkflowId;

    fn create_run(&self, workflow_id: Self::WorkflowId) -> Self::RunId;

    fn run(&self, run_id: Self::RunId);

    fn add_task(
        &self,
        task_name: &str,
        task: TaskImpl<C>,
        pack: Option<PackFn>,
        unpack: Option<UnpackFn>,
    );
}
