use namu_core::{DynamicTaskContext, Task, ir::Workflow};
use std::any::Any;
use std::sync::Arc;

use crate::context::ContextManager;

pub mod simple_engine;

/// Type alias for a function that packs a vector of dynamically typed values (coming
/// from the [`ContextManager`] as `Arc<dyn Any + Send + Sync>`) into a single boxed
/// value that matches the concrete `Input` type of a task.
pub type PackFn = fn(Vec<Arc<dyn Any + Send + Sync>>) -> Box<dyn Any + Send>;

pub trait Engine<C: ContextManager> {
    type WorkflowId;
    type RunId;

    fn create_workflow(&self, workflow: Workflow) -> Self::WorkflowId;

    fn create_run(&self, workflow_id: Self::WorkflowId) -> Self::RunId;

    fn start_run(&self, run_id: Self::RunId);

    fn add_task(
        &self,
        task_name: &str,
        task: Box<dyn Task<C::ContextId, DynamicTaskContext<C::ContextId>> + Send>,
        pack: PackFn,
    );
}
