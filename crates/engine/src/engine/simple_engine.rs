use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use anyhow::Result;
use itertools::Itertools;
use kanal::{Receiver, Sender, unbounded};
use namu_core::ir::{Next, Workflow};
use namu_core::{DynamicTaskContext, Value, ValueId};
use scc::HashIndex;
use scc::ebr::Guard;

use crate::context::ContextManager;
use crate::engine::{Engine, PackFn, TaskImpl, UnpackFn};

pub struct SimpleEngineInner<C: ContextManager> {
    context_manager: C,
    workflows: HashIndex<usize, Workflow>,
    workflow_counter: AtomicUsize,
    runs: HashIndex<usize, usize>,
    run_counter: AtomicUsize,
    tasks: HashIndex<String, TaskImpl<C>>,
    pack_map: HashIndex<String, PackFn>,
    unpack_map: HashIndex<String, UnpackFn>,
    run_results: HashIndex<usize, Receiver<Value>>,
}

pub struct SimpleEngine<C: ContextManager> {
    inner: Arc<SimpleEngineInner<C>>,
}

impl<C: ContextManager + Send + Sync + 'static> SimpleEngine<C> {
    pub fn new(context_manager: C) -> SimpleEngine<C> {
        SimpleEngine {
            inner: Arc::new(SimpleEngineInner {
                context_manager,
                workflows: HashIndex::new(),
                workflow_counter: AtomicUsize::new(0),
                runs: HashIndex::new(),
                run_counter: AtomicUsize::new(0),
                tasks: HashIndex::new(),
                pack_map: HashIndex::new(),
                unpack_map: HashIndex::new(),
                run_results: HashIndex::new(),
            }),
        }
    }
}

impl<C: ContextManager + Send + Sync + 'static> Engine<C> for SimpleEngine<C> {
    type WorkflowId = usize;
    type RunId = usize;

    fn create_workflow(&self, workflow: Workflow) -> Self::WorkflowId {
        let id = self.inner.workflow_counter.fetch_add(1, Ordering::Release);
        if self.inner.workflows.get(&id).is_some() {
            eprintln!("[engine] Replaced existing workflow with id {id}");
            self.inner.workflows.remove(&id);
        }
        self.inner.workflows.insert(id, workflow).unwrap();
        id
    }

    fn create_run(&self, workflow_id: Self::WorkflowId) -> Self::RunId {
        let id = self.inner.run_counter.fetch_add(1, Ordering::Release);
        if self.inner.runs.get(&id).is_some() {
            eprintln!("[engine] Replaced existing run mapping with id {id}");
            self.inner.runs.remove(&id);
        }
        self.inner.runs.insert(id, workflow_id).unwrap();
        id
    }

    fn run(&self, run_id: Self::RunId) {
        let guard = Guard::new();
        let workflow_id = *self.inner.runs.peek(&run_id, &guard).unwrap();
        let workflow = self
            .inner
            .workflows
            .peek(&workflow_id, &guard)
            .cloned()
            .unwrap();
        drop(guard);

        let task_senders = Arc::new(HashIndex::new());
        let ctx_origin: HashIndex<C::ContextId, usize> = HashIndex::new();
        let (result_tx, result_rx) = unbounded::<Value>();
        self.inner.run_results.insert(run_id, result_rx).unwrap();
        let ctx_origin_ref = &ctx_origin;
        let workflow_ref = &workflow;
        let result_tx_ref = &result_tx;

        thread::scope(|s| {
            workflow_ref
                .operations
                .iter()
                .map(|op| op.call.as_ref().unwrap().task_id.clone())
                .unique()
                .collect_vec()
                .into_iter()
                .for_each(|task_name| {
                    let task_senders = Arc::clone(&task_senders);

                    let (in_tx, in_rx) = unbounded::<(C::ContextId, Value)>();
                    let (out_tx, out_rx) = unbounded::<(C::ContextId, Result<Value>)>();

                    task_senders
                        .insert(task_name.clone(), in_tx.clone())
                        .unwrap();

                    let guard = Guard::new();
                    let task = self.inner.tasks.peek(&task_name, &guard).cloned().unwrap();
                    drop(guard);

                    s.spawn(move || {
                        let mut task = task;

                        if let Err(e) = task.prepare() {
                            todo!("prepare error: {e}");
                        }

                        if let Err(e) = task.run(DynamicTaskContext::new(in_rx, out_tx)) {
                            todo!("run error: {e}");
                        }
                    });

                    s.spawn(move || {
                        while let Ok((ctx_id, out_box)) = out_rx.recv() {
                            match out_box {
                                Ok(res) => {
                                    let guard = Guard::new();
                                    let origin_op_id =
                                        *ctx_origin_ref.peek(&ctx_id, &guard).unwrap();
                                    let operation = &workflow_ref.operations[origin_op_id];
                                    let call =
                                        operation.call.as_ref().expect("origin should be call");
                                    let unpack_fn =
                                        self.inner.unpack_map.peek(&call.task_id, &guard).cloned();
                                    drop(guard);

                                    let ctx_id = if let Some(unpack_fn) = unpack_fn {
                                        let out_vals = (unpack_fn)(res);
                                        add_values(
                                            &self.inner.context_manager,
                                            ctx_id,
                                            call.outputs.iter().copied().zip(out_vals).collect(),
                                        )
                                    } else {
                                        self.inner.context_manager.add_value(
                                            ctx_id,
                                            call.outputs[0],
                                            res,
                                        )
                                    };

                                    if let Some(next_op_id) = next_op_id(
                                        &operation.next,
                                        ctx_id.clone(),
                                        &self.inner.context_manager,
                                        result_tx_ref,
                                    ) {
                                        drive_until_call(
                                            &self.inner.context_manager,
                                            workflow_ref,
                                            next_op_id,
                                            Some(origin_op_id),
                                            ctx_id.clone(),
                                            &*task_senders,
                                            &self.inner.pack_map,
                                            ctx_origin_ref,
                                            result_tx_ref,
                                        );
                                    }
                                }
                                Err(err) => {
                                    if !err.is::<namu_core::TaskEnd>() {
                                        eprintln!("[dispatcher::{task_name}] error: {err}");
                                    } else {
                                        self.inner.context_manager.remove_context(ctx_id);
                                    }
                                }
                            }
                        }
                    });
                });

            // Kick off execution for the entry operation (op id 0).
            let root_ctx = self.inner.context_manager.create_context();
            drive_until_call(
                &self.inner.context_manager,
                workflow_ref,
                0,
                None,
                root_ctx,
                &*task_senders,
                &self.inner.pack_map,
                ctx_origin_ref,
                result_tx_ref,
            );
        });
    }

    fn add_task(
        &self,
        task_name: &str,
        task: TaskImpl<C>,
        pack: Option<PackFn>,
        unpack: Option<UnpackFn>,
    ) {
        if self.inner.tasks.get(task_name).is_some() {
            eprintln!("[engine] Replaced existing task registration for '{task_name}'");
            self.inner.tasks.remove(task_name);
        }
        let _ = self.inner.tasks.insert(task_name.to_string(), task);
        if let Some(pack) = pack {
            let _ = self.inner.pack_map.insert(task_name.to_string(), pack);
        }
        if let Some(unpack) = unpack {
            let _ = self.inner.unpack_map.insert(task_name.to_string(), unpack);
        }
    }
}

fn add_values<C: ContextManager>(
    context_manager: &C,
    ctx_id: C::ContextId,
    vals: Vec<(ValueId, Value)>,
) -> C::ContextId {
    let mut iter = vals.into_iter();
    let (val_id, val_arc) = iter.next().unwrap();
    let mut new_ctx_id = context_manager.add_value(ctx_id, val_id, val_arc);
    for (val_id, val_arc) in iter {
        let old_ctx_id = new_ctx_id;
        new_ctx_id = context_manager.add_value(old_ctx_id.clone(), val_id, val_arc);
        context_manager.remove_context(old_ctx_id);
    }
    new_ctx_id
}

fn next_op_id<C: ContextManager>(
    next: &Next,
    ctx_id: C::ContextId,
    context_manager: &C,
    result: &Sender<Value>,
) -> Option<usize>
where
    C::ContextId: Clone,
{
    match next {
        Next::Jump { next } => Some(*next),
        Next::Branch {
            var,
            true_next,
            false_next,
        } => {
            let cond_any = context_manager.get_value(ctx_id.clone(), *var);
            // unwrap is safe because we check statically that the condition is a bool
            let cond = cond_any.downcast_ref::<bool>().copied().unwrap();
            Some(if cond { *true_next } else { *false_next })
        }
        Next::Return { var } => {
            if let Some(v_id) = var {
                let val = context_manager.get_value(ctx_id.clone(), *v_id);
                let _ = result.send(val);
            } else {
                let _ = result.send(Value::new(()));
            }

            context_manager.remove_context(ctx_id);
            None
        }
    }
}

impl<C: ContextManager + Send + Sync + 'static> SimpleEngine<C> {
    pub fn get_result(&self, run_id: usize) -> Receiver<Value> {
        let guard = Guard::new();
        let slot = self
            .inner
            .run_results
            .peek(&run_id, &guard)
            .unwrap()
            .clone();
        drop(guard);
        slot
    }
}

// ---------------- Helper utilities ----------------------------------------

fn parse_literal(raw: &str) -> Value {
    match raw {
        "true" => Value::new(true),
        "false" => Value::new(false),
        "()" => Value::new(()),
        _ => {
            if let Ok(n) = i32::from_str(raw) {
                Value::new(n)
            } else {
                // Best-effort: treat everything else as String (strip quotes)
                let s = raw.trim_matches('"').to_string();
                Value::new(s)
            }
        }
    }
}

/// Walk the workflow starting at `op_id`, executing literals & φ, and schedule the first `Call`.
/// Returns `Some(updated_ctx_id)` when a call is enqueued, or `None` if a Return terminator was hit.
fn drive_until_call<C: ContextManager>(
    context_manager: &C,
    workflow: &Workflow,
    mut op_id: usize,
    mut pred_op: Option<usize>,
    mut ctx_id: C::ContextId,
    task_senders: &HashIndex<String, Sender<(C::ContextId, Value)>>,
    pack_map: &HashIndex<String, PackFn>,
    ctx_origin: &HashIndex<C::ContextId, usize>,
    result_tx: &Sender<Value>,
) -> Option<C::ContextId>
where
    C::ContextId: Clone,
{
    loop {
        let operation = &workflow.operations[op_id];

        // --- evaluate literals --------------------------------------------------------
        if !operation.literals.is_empty() {
            let lit_vals = operation
                .literals
                .iter()
                .map(|lit| (lit.output, parse_literal(&lit.value)))
                .collect();
            ctx_id = add_values(context_manager, ctx_id, lit_vals);
        }

        // --- evaluate φ nodes ---------------------------------------------------------
        if !operation.phis.is_empty() {
            let mut phi_vals = Vec::with_capacity(operation.phis.len());
            for phi in &operation.phis {
                if let Some(pred) = pred_op {
                    if let Some((_, val_id)) = phi.from.iter().find(|(from_id, _)| *from_id == pred)
                    {
                        let val = context_manager.get_value(ctx_id.clone(), *val_id);
                        phi_vals.push((phi.output, val));
                    }
                }
            }
            if !phi_vals.is_empty() {
                ctx_id = add_values(context_manager, ctx_id, phi_vals);
            }
        }

        // --- if this op invokes a task, enqueue it and stop ---------------------------
        if let Some(call) = &operation.call {
            let guard = Guard::new();

            // Gather inputs
            let inputs = context_manager.get_values(ctx_id.clone(), &call.inputs);

            // Pack inputs if necessary
            let packed = if let Some(pack_fn) = pack_map.peek(&call.task_id, &guard).cloned() {
                (pack_fn)(inputs)
            } else {
                debug_assert_eq!(
                    inputs.len(),
                    1,
                    "Task '{}' missing pack fn and inputs >1",
                    call.task_id
                );
                inputs.into_iter().next().unwrap()
            };

            // Send to task
            let sender = task_senders
                .peek(&call.task_id, &guard)
                .expect("sender not found")
                .clone();
            drop(guard);

            sender
                .send((ctx_id.clone(), packed))
                .expect("failed to send to task");

            // Register origin op for dispatcher
            let _ = ctx_origin.insert(ctx_id.clone(), op_id);

            return Some(ctx_id);
        }

        // --- otherwise follow control-flow -------------------------------------------
        match next_op_id(&operation.next, ctx_id.clone(), context_manager, result_tx) {
            Some(next) => {
                pred_op = Some(op_id);
                op_id = next;
            }
            None => return None, // Workflow returned
        }
    }
}
