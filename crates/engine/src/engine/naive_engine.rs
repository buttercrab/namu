use std::sync::atomic::{AtomicUsize, Ordering};

use namu_core::{Task, ir::Workflow};
use scc::HashMap;

use crate::{context::ContextManager, engine::Engine};

pub struct NaiveEngine<C: ContextManager> {
    context_manager: C,
    workflows: HashMap<usize, Workflow>,
    workflow_counter: AtomicUsize,
    runs: HashMap<usize, usize>,
    run_counter: AtomicUsize,
    tasks: HashMap<usize, Box<dyn Task<C::ContextId, C>>>,
}

impl<C: ContextManager> NaiveEngine<C> {
    fn new(context_manager: C) -> NaiveEngine<C> {
        NaiveEngine {
            context_manager,
            workflows: HashMap::new(),
            workflow_counter: AtomicUsize::new(0),
            runs: HashMap::new(),
            run_counter: AtomicUsize::new(0),
            tasks: HashMap::new(),
        }
    }
}

impl<C: ContextManager> Engine<C> for NaiveEngine<C> {
    type WorkflowId = usize;

    type RunId = usize;

    fn create_workflow(&self, workflow: Workflow) -> Self::WorkflowId {
        let id = self.workflow_counter.fetch_add(1, Ordering::Release);
        self.workflows.insert(id, workflow).unwrap();
        id
    }

    fn create_run(&self, workflow_id: Self::WorkflowId) -> Self::RunId {
        let id = self.run_counter.fetch_add(1, Ordering::Release);
        self.runs.insert(id, workflow_id).unwrap();
        id
    }

    fn start_run(&self, run_id: Self::RunId) {
        todo!()
    }
}
