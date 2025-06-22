use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use inventory;
use itertools::Itertools;
use kanal::{Receiver, Sender as OneShotSender, Sender, bounded, unbounded};
use namu_core::ir::{Next, Workflow};
use namu_core::registry::{PackFn, TaskEntry, TaskImpl, UnpackFn};
use namu_core::{ContextId, DynamicTaskContext, Value, ValueId};
use scc::ebr::Guard;
use scc::{HashIndex, HashMap};

use crate::context::ContextManager;
use crate::engine::Engine;

struct RunContext<'a, C: ContextManager> {
    context_manager: &'a C,
    workflow: &'a Workflow,
    ctx_origin: &'a HashIndex<ContextId, usize>,
    result_tx: &'a Sender<Value>,
    active_ctxs: &'a AtomicUsize,
    finish_tx: &'a OneShotSender<()>,
    task_senders: &'a HashIndex<String, Sender<(ContextId, Value)>>,
    pack_map: &'a HashIndex<String, PackFn>,
}

impl<'a, C: ContextManager> Clone for RunContext<'a, C> {
    fn clone(&self) -> Self {
        RunContext {
            context_manager: self.context_manager,
            workflow: self.workflow,
            ctx_origin: self.ctx_origin,
            result_tx: self.result_tx,
            active_ctxs: self.active_ctxs,
            finish_tx: self.finish_tx,
            task_senders: self.task_senders,
            pack_map: self.pack_map,
        }
    }
}

pub struct SimpleEngineInner<C: ContextManager> {
    context_manager: C,
    workflows: HashIndex<usize, Workflow>,
    workflow_counter: AtomicUsize,
    runs: HashIndex<usize, usize>,
    run_counter: AtomicUsize,
    tasks: HashIndex<String, TaskImpl>,
    pack_map: HashIndex<String, PackFn>,
    unpack_map: HashIndex<String, UnpackFn>,
    run_results: HashMap<usize, Receiver<Value>>,
    run_result_senders: HashMap<usize, Sender<Value>>,
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
                run_results: HashMap::new(),
                run_result_senders: HashMap::new(),
            }),
        }
    }

    /// Convenience helper that constructs the engine and automatically
    /// registers every task collected via the `inventory` registry.  This is
    /// the common way end-users interact with the runtime – they no longer
    /// need to call `add_task` manually.
    pub fn with_registered(context_manager: C) -> SimpleEngine<C> {
        let engine = Self::new(context_manager);
        engine.register_all_tasks();
        engine
    }

    /// Iterate over the global task registry and add each entry to the
    /// engine's internal maps.
    fn register_all_tasks(&self) {
        for entry in inventory::iter::<TaskEntry> {
            let task: TaskImpl = (entry.create)();
            self.add_task(entry.name, task, entry.pack, entry.unpack);
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

        // Pre-create result channel so `get_result` can be called before `run` starts.
        let (result_tx, result_rx) = unbounded();
        self.inner.run_results.insert(id, result_rx).unwrap();
        self.inner.run_result_senders.insert(id, result_tx).unwrap();
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
        let result_tx = self.inner.run_result_senders.get(&run_id).unwrap().clone();

        let task_senders = HashIndex::new();
        let ctx_origin: HashIndex<ContextId, usize> = HashIndex::new();

        // Finish signalling primitives (one-shot)
        let (finish_tx, finish_rx) = bounded::<()>(1);
        let active_ctxs = AtomicUsize::new(0);

        thread::scope(|s| {
            let run_ctx = RunContext {
                context_manager: &self.inner.context_manager,
                workflow: &workflow,
                ctx_origin: &ctx_origin,
                result_tx: &result_tx,
                active_ctxs: &active_ctxs,
                finish_tx: &finish_tx,
                task_senders: &task_senders,
                pack_map: &self.inner.pack_map,
            };

            workflow
                .operations
                .iter()
                .filter_map(|op| op.call.as_ref().map(|call| call.task_id.clone()))
                .unique()
                .collect_vec()
                .into_iter()
                .for_each(|task_name| {
                    let (in_tx, in_rx) = unbounded();
                    let (out_tx, out_rx) = unbounded();

                    run_ctx
                        .task_senders
                        .insert(task_name.clone(), in_tx)
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

                    let run_ctx = run_ctx.clone();
                    s.spawn(move || {
                        while let Ok((ctx_id, out_box)) = out_rx.recv() {
                            match out_box {
                                Ok(res) => {
                                    let guard = Guard::new();
                                    let origin_op_id =
                                        *run_ctx.ctx_origin.peek(&ctx_id, &guard).unwrap();
                                    drop(guard);

                                    let operation = &run_ctx.workflow.operations[origin_op_id];
                                    let call =
                                        operation.call.as_ref().expect("origin should be call");

                                    let guard = Guard::new();
                                    let unpack_fn =
                                        self.inner.unpack_map.peek(&call.task_id, &guard).cloned();
                                    drop(guard);

                                    let ctx_id = if let Some(unpack_fn) = unpack_fn {
                                        let out_vals = (unpack_fn)(res);
                                        add_values(
                                            &run_ctx,
                                            ctx_id,
                                            call.outputs.iter().copied().zip(out_vals).collect(),
                                        )
                                    } else {
                                        let new_id = self.inner.context_manager.add_value(
                                            ctx_id,
                                            call.outputs[0],
                                            res,
                                        );
                                        run_ctx.active_ctxs.fetch_add(1, Ordering::Release);
                                        new_id
                                    };

                                    if let Some(next_op_id) =
                                        next_op_id(&run_ctx, &operation.next, ctx_id)
                                    {
                                        drive_until_call(
                                            &run_ctx,
                                            next_op_id,
                                            Some(origin_op_id),
                                            ctx_id,
                                        );
                                    }
                                }
                                Err(err) => {
                                    if !err.is::<namu_core::TaskEnd>() {
                                        eprintln!("[dispatcher::{task_name}] error: {err}");
                                    }
                                    self.inner.context_manager.remove_context(ctx_id);
                                    if run_ctx.active_ctxs.fetch_sub(1, Ordering::AcqRel) == 1 {
                                        let _ = run_ctx.finish_tx.send(());
                                    }
                                }
                            }
                        }
                    });
                });

            // Kick off execution for the entry operation (op id 0).
            let root_ctx = self.inner.context_manager.create_context();
            run_ctx.active_ctxs.fetch_add(1, Ordering::Release);
            drive_until_call(&run_ctx, 0, None, root_ctx);

            // Wait until every context is dropped then clear senders so task threads exit.
            let _ = finish_rx.recv();
            let guard = Guard::new();
            run_ctx.task_senders.iter(&guard).for_each(|(_, tx)| {
                let _ = tx.close();
            });
        });
        drop(result_tx);

        let _ = self.inner.run_results.remove(&run_id);
        let _ = self.inner.run_result_senders.remove(&run_id);
        self.inner.runs.remove(&run_id);
    }

    fn add_task(
        &self,
        task_name: &str,
        task: TaskImpl,
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
    run_ctx: &RunContext<C>,
    ctx_id: ContextId,
    vals: Vec<(ValueId, Value)>,
) -> ContextId {
    debug_assert!(!vals.is_empty());
    let mut iter = vals.into_iter();
    let (val_id, val_arc) = iter.next().unwrap();
    let mut new_ctx_id = run_ctx.context_manager.add_value(ctx_id, val_id, val_arc);
    for (val_id, val_arc) in iter {
        let old_ctx_id = new_ctx_id;
        new_ctx_id = run_ctx
            .context_manager
            .add_value(old_ctx_id, val_id, val_arc);
        run_ctx.context_manager.remove_context(old_ctx_id);
    }

    run_ctx.active_ctxs.fetch_add(1, Ordering::Release);
    new_ctx_id
}

fn next_op_id<C: ContextManager>(
    run_ctx: &RunContext<C>,
    next: &Next,
    ctx_id: ContextId,
) -> Option<usize> {
    match next {
        Next::Jump { next } => Some(*next),
        Next::Branch {
            var,
            true_next,
            false_next,
        } => {
            let cond_any = run_ctx.context_manager.get_value(ctx_id, *var);
            // unwrap is safe because we check statically that the condition is a bool
            let cond = cond_any.downcast_ref::<bool>().copied().unwrap();
            Some(if cond { *true_next } else { *false_next })
        }
        Next::Return { var } => {
            if let Some(v_id) = var {
                let val = run_ctx.context_manager.get_value(ctx_id, *v_id);
                let _ = run_ctx.result_tx.send(val);
            } else {
                let _ = run_ctx.result_tx.send(Value::new(()));
            }

            run_ctx.context_manager.remove_context(ctx_id);
            if run_ctx.active_ctxs.fetch_sub(1, Ordering::AcqRel) == 1 {
                let _ = run_ctx.finish_tx.send(());
            }
            None
        }
    }
}

impl<C: ContextManager + Send + Sync + 'static> SimpleEngine<C> {
    pub fn get_result(&self, run_id: usize) -> Receiver<Value> {
        self.inner.run_results.get(&run_id).unwrap().clone()
    }
}

impl<C: ContextManager + Send + Sync + 'static> Clone for SimpleEngine<C> {
    fn clone(&self) -> Self {
        SimpleEngine {
            inner: self.inner.clone(),
        }
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
/// Returns `Some(updated_ctx_id)` when a call is enqueued, or `None` if a Return terminator was
/// hit.
fn drive_until_call<C: ContextManager>(
    run_ctx: &RunContext<C>,
    mut op_id: usize,
    mut pred_op: Option<usize>,
    mut ctx_id: ContextId,
) -> Option<ContextId> {
    loop {
        let operation = &run_ctx.workflow.operations[op_id];

        let mut values_to_add = Vec::new();

        if !operation.literals.is_empty() {
            let lit_vals = operation
                .literals
                .iter()
                .map(|lit| (lit.output, parse_literal(&lit.value)));
            values_to_add.extend(lit_vals);
        }

        if !operation.phis.is_empty() {
            if let Some(pred) = pred_op {
                let phi_vals = operation.phis.iter().filter_map(|phi| {
                    phi.from
                        .iter()
                        .find(|(from_id, _)| *from_id == pred)
                        .map(|(_, val_id)| {
                            (
                                phi.output,
                                run_ctx.context_manager.get_value(ctx_id, *val_id),
                            )
                        })
                });
                values_to_add.extend(phi_vals);
            }
        }

        if !values_to_add.is_empty() {
            let old_ctx_id = ctx_id;
            ctx_id = add_values(run_ctx, ctx_id, values_to_add);

            run_ctx.context_manager.remove_context(old_ctx_id);
            if run_ctx.active_ctxs.fetch_sub(1, Ordering::AcqRel) == 1 {
                let _ = run_ctx.finish_tx.send(());
            }
        }

        if let Some(call) = &operation.call {
            let inputs = run_ctx.context_manager.get_values(ctx_id, &call.inputs);

            let guard = Guard::new();
            let pack_fn = run_ctx.pack_map.peek(&call.task_id, &guard).cloned();
            drop(guard);

            let packed = if let Some(pack_fn) = pack_fn {
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

            let guard = Guard::new();
            let sender = run_ctx
                .task_senders
                .peek(&call.task_id, &guard)
                .expect("sender not found")
                .clone();
            drop(guard);

            let _ = run_ctx.ctx_origin.insert(ctx_id, op_id);

            sender
                .send((ctx_id, packed))
                .expect("failed to send to task");

            return Some(ctx_id);
        }

        match next_op_id(run_ctx, &operation.next, ctx_id) {
            Some(next) => {
                pred_op = Some(op_id);
                op_id = next;
            }
            None => return None, // Workflow returned
        }
    }
}
