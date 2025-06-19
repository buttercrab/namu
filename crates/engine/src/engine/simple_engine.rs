use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use namu_core::{DynamicTaskContext, Task, ir::Workflow};
use scc::HashMap as SccHashMap;
use std::collections::HashMap;

use crate::{
    context::ContextManager,
    engine::{Engine, PackFn},
};

pub struct NaiveEngine<C: ContextManager> {
    context_manager: Arc<C>,
    workflows: Mutex<HashMap<usize, Arc<Workflow>>>,
    workflow_counter: AtomicUsize,
    runs: Mutex<HashMap<usize, usize>>,
    run_counter: AtomicUsize,
    tasks: Mutex<
        HashMap<
            String,
            (
                Arc<Mutex<Box<dyn Task<C::ContextId, DynamicTaskContext<C::ContextId>> + Send>>>,
                PackFn,
            ),
        >,
    >,
}

impl<C: ContextManager + Send + Sync + 'static> NaiveEngine<C> {
    pub fn new(context_manager: C) -> NaiveEngine<C> {
        NaiveEngine {
            context_manager: Arc::new(context_manager),
            workflows: Mutex::new(HashMap::new()),
            workflow_counter: AtomicUsize::new(0),
            runs: Mutex::new(HashMap::new()),
            run_counter: AtomicUsize::new(0),
            tasks: Mutex::new(HashMap::new()),
        }
    }
}

impl<C: ContextManager + Send + Sync + 'static> Engine<C> for NaiveEngine<C> {
    type WorkflowId = usize;

    type RunId = usize;

    fn create_workflow(&self, workflow: Workflow) -> Self::WorkflowId {
        let id = self.workflow_counter.fetch_add(1, Ordering::Release);
        self.workflows
            .lock()
            .unwrap()
            .insert(id, Arc::new(workflow))
            .unwrap();
        id
    }

    fn create_run(&self, workflow_id: Self::WorkflowId) -> Self::RunId {
        let id = self.run_counter.fetch_add(1, Ordering::Release);
        self.runs.lock().unwrap().insert(id, workflow_id).unwrap();
        id
    }

    fn start_run(&self, run_id: Self::RunId) {
        use anyhow::Result;
        use kanal::{Receiver, Sender, unbounded};
        use std::thread;

        // Retrieve the workflow associated with this run.
        let Some(&workflow_id) = self.runs.lock().unwrap().get(&run_id) else {
            eprintln!("[engine] Unknown run_id {run_id}");
            return;
        };

        let Some(workflow) = self.workflows.lock().unwrap().get(&workflow_id).cloned() else {
            eprintln!("[engine] No workflow for id {workflow_id}");
            return;
        };

        // Local helper alias.
        type BoxedAnySend = Box<dyn Any + Send>;

        // Map from task name to its input sender channel.
        let task_senders: Arc<
            Mutex<std::collections::HashMap<String, Sender<(C::ContextId, BoxedAnySend)>>>,
        > = Arc::new(Mutex::new(std::collections::HashMap::new()));

        // Map from context id to originating op id (Call).
        let ctx_origin: Arc<SccHashMap<C::ContextId, usize>> = Arc::new(SccHashMap::new());

        // Spawn worker & dispatcher threads for each registered task.
        for (task_name, (task_arc, pack_fn)) in self.tasks.lock().unwrap().iter() {
            // Create channels for this task.
            let (in_tx, in_rx): (
                Sender<(C::ContextId, BoxedAnySend)>,
                Receiver<(C::ContextId, BoxedAnySend)>,
            ) = unbounded();
            let (out_tx, out_rx): (
                Sender<(C::ContextId, Result<BoxedAnySend>)>,
                Receiver<(C::ContextId, Result<BoxedAnySend>)>,
            ) = unbounded();

            task_senders
                .lock()
                .unwrap()
                .insert(task_name.clone(), in_tx.clone());

            // Clone needed references for worker thread.
            let task_arc = Arc::clone(task_arc);
            let task_name_clone = task_name.clone();

            thread::spawn(move || {
                let mut task_guard = task_arc.lock().expect("Task mutex poisoned");
                if let Err(e) = task_guard.prepare() {
                    eprintln!("[task::{task_name_clone}] prepare error: {e}");
                    return;
                }
                if let Err(e) = task_guard.run(DynamicTaskContext::new(in_rx, out_tx)) {
                    eprintln!("[task::{task_name_clone}] run error: {e}");
                }
            });

            // Dispatcher thread.
            let workflow_clone = Arc::clone(&workflow);
            let ctx_origin_clone = Arc::clone(&ctx_origin);
            let task_name_disp = task_name.clone();
            let task_senders_clone = Arc::clone(&task_senders);
            let context_manager_ref = Arc::clone(&self.context_manager);
            thread::spawn(move || {
                while let Ok((ctx_id, res)) = out_rx.recv() {
                    match res {
                        Ok(val_box) => {
                            // Handle normal output.
                            process_task_output::<C>(
                                &workflow_clone,
                                &context_manager_ref,
                                &task_senders_clone,
                                &ctx_origin_clone,
                            );
                        }
                        Err(err) => {
                            // Treat TaskEnd specially; for now, ignore other errors.
                            if err.is::<namu_core::TaskEnd>() {
                                // context end - remove ctx maybe
                            } else {
                                eprintln!("[dispatcher::{task_name_disp}] error");
                            }
                        }
                    }
                }
            });
        }

        // Kick off execution with a fresh context starting at op 0.
        let ctx_id = self.context_manager.create_context();
        execute_from_op(
            &workflow,
            0,
            ctx_id,
            self.context_manager.clone(),
            &task_senders,
            &ctx_origin,
        );

        // NOTE: in this naive implementation we do not track live threads to join.
        // Threads will run until all contexts reach Return and channels close.
    }

    fn add_task(
        &self,
        task_name: &str,
        task: Box<dyn Task<C::ContextId, DynamicTaskContext<C::ContextId>> + Send>,
        pack: PackFn,
    ) {
        let task_arc = Arc::new(Mutex::new(task));
        let _ = self
            .tasks
            .lock()
            .unwrap()
            .insert(task_name.to_string(), (task_arc, pack));
    }
}

// --- Helper stub implementations ------------------------------------------------

use kanal::Sender;

type BoxedAnySend = Box<dyn Any + Send>;

#[allow(clippy::too_many_arguments)]
fn process_task_output<C: ContextManager>(
    _workflow: &Arc<Workflow>,
    _context_manager: &Arc<C>,
    _task_senders: &Arc<
        Mutex<std::collections::HashMap<String, Sender<(C::ContextId, BoxedAnySend)>>>,
    >,
    _ctx_origin: &Arc<SccHashMap<C::ContextId, usize>>,
) {
    // For now, simply print that we received an output and mark context as completed.
    println!("[engine] Received output for a context");
}

fn execute_from_op<C>(
    workflow: &Arc<Workflow>,
    op_id: usize,
    ctx_id: C::ContextId,
    context_manager: Arc<C>,
    task_senders: &Arc<
        Mutex<std::collections::HashMap<String, Sender<(C::ContextId, BoxedAnySend)>>>,
    >,
    ctx_origin: &Arc<SccHashMap<C::ContextId, usize>>,
) where
    C: ContextManager,
    C::ContextId: Clone,
{
    // Naive, single-threaded traversal starting from `op_id`.

    let mut current_op_id = op_id;
    let mut current_ctx_id = ctx_id;

    loop {
        // Defensive check.
        if current_op_id >= workflow.operations.len() {
            eprintln!(
                "[engine] Invalid op id {current_op_id} (workflow has {} ops)",
                workflow.operations.len()
            );
            break;
        }

        let op = &workflow.operations[current_op_id];

        // 1. Process literals
        for lit in &op.literals {
            let lit_arc: Arc<dyn Any + Send + Sync> = if let Ok(i) = lit.value.parse::<i64>() {
                Arc::new(i)
            } else if let Ok(f) = lit.value.parse::<f64>() {
                Arc::new(f)
            } else if let Ok(b) = lit.value.parse::<bool>() {
                Arc::new(b)
            } else {
                Arc::new(lit.value.clone())
            };

            current_ctx_id = context_manager.add_value(current_ctx_id.clone(), lit.output, lit_arc);
        }

        // 2. Process phi nodes
        for phi in &op.phis {
            let chosen_val_id = if let Some(pred_op) = ctx_origin.get(&current_ctx_id).map(|e| *e) {
                phi.from
                    .iter()
                    .find(|(pred, _)| *pred == pred_op)
                    .map(|(_, v)| *v)
                    .or_else(|| phi.from.first().map(|(_, v)| *v))
                    .unwrap()
            } else {
                phi.from.first().map(|(_, v)| *v).unwrap()
            };

            let value = context_manager.get_value(current_ctx_id.clone(), chosen_val_id);
            current_ctx_id = context_manager.add_value(current_ctx_id.clone(), phi.output, value);
        }

        // 3. Optional call handling
        if let Some(call) = &op.call {
            // Gather input values.
            let vals = context_manager.get_values(current_ctx_id.clone(), &call.inputs);

            let sender_opt = task_senders.lock().unwrap().get(&call.task_id).cloned();

            match sender_opt {
                Some(sender) => {
                    let boxed_inputs: BoxedAnySend = Box::new(vals);

                    let _ = ctx_origin.insert(current_ctx_id.clone(), current_op_id);

                    if let Err(e) = sender.send((current_ctx_id.clone(), boxed_inputs)) {
                        eprintln!(
                            "[engine] Failed to send inputs to task {}: {}",
                            call.task_id, e
                        );
                    }
                }
                None => {
                    eprintln!("[engine] No registered sender for task {}", call.task_id);
                }
            }

            // Call suspends execution until task completes.
            break;
        }

        // Follow control-flow terminator for non-Call operations.
        match &op.next {
            namu_core::ir::Next::Jump { next } => {
                current_op_id = *next;
                continue;
            }
            namu_core::ir::Next::Branch {
                var,
                true_next,
                false_next,
            } => {
                let cond_any = context_manager.get_value(current_ctx_id.clone(), *var);
                let cond = cond_any.downcast_ref::<bool>().copied().unwrap_or(false);
                current_op_id = if cond { *true_next } else { *false_next };
                continue;
            }
            namu_core::ir::Next::Return { var } => {
                if let Some(v_id) = var {
                    let _ = context_manager.get_value(current_ctx_id.clone(), *v_id);
                }
                context_manager.remove_context(current_ctx_id.clone());
                break;
            }
        }
    }
}
