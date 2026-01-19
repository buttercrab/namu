use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use async_trait::async_trait;
use inventory;
use itertools::Itertools;
use kanal::{Receiver, Sender as OneShotSender, Sender, bounded, unbounded};
use namu_core::ir::Workflow;
use namu_core::registry::{PackFn, TaskEntry, TaskImpl, UnpackFn};
use namu_core::{ContextId, DynamicTaskContext, Value};
use scc::ebr::Guard;
use scc::{HashIndex, HashMap};

use crate::engine::{Engine, TaskRegistry};
use crate::kernel::{CallSpec, CoreValueCodec, EngineKernel, KernelAction, ValueStore};
use crate::store::InMemoryStore;

struct RunContext<'a> {
    kernel: &'a EngineKernel<CoreValueCodec>,
    store: &'a InMemoryStore<Value>,
    workflow: &'a Workflow,
    ctx_origin: &'a HashIndex<ContextId, usize>,
    finished_ctxs: &'a HashIndex<ContextId, ()>,
    result_tx: &'a Sender<Value>,
    active_ctxs: &'a AtomicUsize,
    finish_tx: &'a OneShotSender<()>,
    task_senders: &'a HashIndex<String, Sender<(ContextId, Value)>>,
    pack_map: &'a HashIndex<String, PackFn>,
    unpack_map: &'a HashIndex<String, UnpackFn>,
}

impl<'a> Clone for RunContext<'a> {
    fn clone(&self) -> Self {
        RunContext {
            kernel: self.kernel,
            store: self.store,
            workflow: self.workflow,
            ctx_origin: self.ctx_origin,
            finished_ctxs: self.finished_ctxs,
            result_tx: self.result_tx,
            active_ctxs: self.active_ctxs,
            finish_tx: self.finish_tx,
            task_senders: self.task_senders,
            pack_map: self.pack_map,
            unpack_map: self.unpack_map,
        }
    }
}

pub struct SimpleEngineInner {
    kernel: EngineKernel<CoreValueCodec>,
    store: InMemoryStore<Value>,
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

pub struct SimpleEngine {
    inner: Arc<SimpleEngineInner>,
}

impl SimpleEngine {
    pub fn new() -> SimpleEngine {
        SimpleEngine {
            inner: Arc::new(SimpleEngineInner {
                kernel: EngineKernel::new(CoreValueCodec),
                store: InMemoryStore::new(),
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

    /// Construct the engine and register tasks collected via `inventory`.
    pub fn with_registered() -> SimpleEngine {
        let engine = Self::new();
        engine.register_all_tasks();
        engine
    }

    fn register_all_tasks(&self) {
        for entry in inventory::iter::<TaskEntry> {
            let task: TaskImpl = (entry.create)();
            self.add_task_sync(entry.name, task, entry.pack, entry.unpack);
        }
    }

    pub fn get_result(&self, run_id: usize) -> Receiver<Value> {
        self.inner.run_results.get(&run_id).unwrap().clone()
    }

    fn add_task_sync(
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

impl Default for SimpleEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Engine for SimpleEngine {
    type WorkflowId = usize;
    type RunId = usize;

    async fn create_workflow(&self, workflow: Workflow) -> Self::WorkflowId {
        let id = self.inner.workflow_counter.fetch_add(1, Ordering::Release);
        if self.inner.workflows.get(&id).is_some() {
            eprintln!("[engine] Replaced existing workflow with id {id}");
            self.inner.workflows.remove(&id);
        }
        self.inner.workflows.insert(id, workflow).unwrap();
        id
    }

    async fn create_run(&self, workflow_id: Self::WorkflowId) -> Self::RunId {
        let id = self.inner.run_counter.fetch_add(1, Ordering::Release);
        if self.inner.runs.get(&id).is_some() {
            eprintln!("[engine] Replaced existing run mapping with id {id}");
            self.inner.runs.remove(&id);
        }
        self.inner.runs.insert(id, workflow_id).unwrap();

        let (result_tx, result_rx) = unbounded();
        self.inner.run_results.insert(id, result_rx).unwrap();
        self.inner.run_result_senders.insert(id, result_tx).unwrap();
        id
    }

    async fn run(&self, run_id: Self::RunId) -> anyhow::Result<()> {
        let workflow_id = *self.inner.runs.peek(&run_id, &Guard::new()).unwrap();
        let workflow = self
            .inner
            .workflows
            .peek(&workflow_id, &Guard::new())
            .cloned()
            .unwrap();
        let result_tx = self.inner.run_result_senders.get(&run_id).unwrap().clone();

        let task_senders = HashIndex::new();
        let ctx_origin: HashIndex<ContextId, usize> = HashIndex::new();
        let finished_ctxs: HashIndex<ContextId, ()> = HashIndex::new();

        let (finish_tx, finish_rx) = bounded::<()>(1);
        let active_ctxs = AtomicUsize::new(0);

        let (event_tx, event_rx) = unbounded::<TaskEvent>();

        let run_ctx = RunContext {
            kernel: &self.inner.kernel,
            store: &self.inner.store,
            workflow: &workflow,
            ctx_origin: &ctx_origin,
            finished_ctxs: &finished_ctxs,
            result_tx: &result_tx,
            active_ctxs: &active_ctxs,
            finish_tx: &finish_tx,
            task_senders: &task_senders,
            pack_map: &self.inner.pack_map,
            unpack_map: &self.inner.unpack_map,
        };

        // Start task workers + output forwarders.
        workflow
            .operations
            .iter()
            .filter_map(|op| op.call.as_ref().map(|call| call.task_id.clone()))
            .unique()
            .for_each(|task_name| {
                let (in_tx, in_rx) = unbounded();
                let (out_tx, out_rx) = unbounded();

                run_ctx
                    .task_senders
                    .insert(task_name.clone(), in_tx)
                    .unwrap();

                let task = self
                    .inner
                    .tasks
                    .peek(&task_name, &Guard::new())
                    .cloned()
                    .unwrap();

                thread::spawn(move || {
                    let mut task = task;
                    if let Err(e) = task.prepare() {
                        panic!("prepare error: {e}");
                    }
                    if let Err(e) = task.run(DynamicTaskContext::new(in_rx, out_tx)) {
                        panic!("run error: {e}");
                    }
                });

                let event_tx = event_tx.clone();
                let task_name_clone = task_name.clone();
                thread::spawn(move || {
                    while let Ok((ctx_id, out_box)) = out_rx.recv() {
                        let _ = event_tx.send(TaskEvent {
                            task_name: task_name_clone.clone(),
                            ctx_id,
                            output: out_box,
                        });
                    }
                });
            });

        // Kick off the root context.
        let root_ctx = self.inner.store.create_root();
        run_ctx.active_ctxs.fetch_add(1, Ordering::Release);
        drive_from(&run_ctx, root_ctx, 0, None).await?;

        let finish_rx_async = finish_rx.as_async();
        let event_rx_async = event_rx.as_async();

        loop {
            tokio::select! {
                _ = finish_rx_async.recv() => break,
                event = event_rx_async.recv() => {
                    match event {
                        Ok(event) => {
                            handle_event(&run_ctx, event).await?;
                        }
                        Err(_) => break,
                    }
                }
            }
        }

        let _ = self.inner.run_results.remove(&run_id);
        let _ = self.inner.run_result_senders.remove(&run_id);
        self.inner.runs.remove(&run_id);

        Ok(())
    }
}

#[async_trait]
impl TaskRegistry for SimpleEngine {
    async fn add_task(
        &self,
        task_name: &str,
        task: TaskImpl,
        pack: Option<PackFn>,
        unpack: Option<UnpackFn>,
    ) {
        self.add_task_sync(task_name, task, pack, unpack);
    }
}

impl Clone for SimpleEngine {
    fn clone(&self) -> Self {
        SimpleEngine {
            inner: self.inner.clone(),
        }
    }
}

struct TaskEvent {
    task_name: String,
    ctx_id: ContextId,
    output: anyhow::Result<Value>,
}

async fn drive_from(
    run_ctx: &RunContext<'_>,
    ctx_id: ContextId,
    start_op: usize,
    pred_op: Option<usize>,
) -> anyhow::Result<()> {
    match run_ctx
        .kernel
        .drive_until_action(run_ctx.workflow, run_ctx.store, ctx_id, start_op, pred_op)
        .await?
    {
        KernelAction::Dispatch {
            op_id,
            ctx_id,
            call,
        } => {
            dispatch_call(run_ctx, op_id, ctx_id, &call).await?;
        }
        KernelAction::Return { ctx_id, return_var } => {
            if let Some(var) = return_var {
                let val = run_ctx.store.get_value(ctx_id, var).await?;
                let _ = run_ctx.result_tx.send(val);
            } else {
                let _ = run_ctx.result_tx.send(Value::new(()));
            }
            finish_ctx(run_ctx, ctx_id);
        }
    }
    Ok(())
}

async fn dispatch_call(
    run_ctx: &RunContext<'_>,
    op_id: usize,
    ctx_id: ContextId,
    call: &CallSpec,
) -> anyhow::Result<()> {
    let inputs = run_ctx.store.get_values(ctx_id, &call.inputs).await?;

    let pack_fn = run_ctx.pack_map.peek(&call.task_id, &Guard::new()).cloned();

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

    let sender = run_ctx
        .task_senders
        .peek(&call.task_id, &Guard::new())
        .expect("sender not found")
        .clone();

    let _ = run_ctx.ctx_origin.insert(ctx_id, op_id);
    sender
        .send((ctx_id, packed))
        .expect("failed to send to task");
    Ok(())
}

async fn handle_event(run_ctx: &RunContext<'_>, event: TaskEvent) -> anyhow::Result<()> {
    match event.output {
        Ok(res) => {
            let origin_op_id = *run_ctx
                .ctx_origin
                .peek(&event.ctx_id, &Guard::new())
                .expect("origin not found");
            let operation = &run_ctx.workflow.operations[origin_op_id];
            let call = operation.call.as_ref().expect("origin should be call");

            let unpack_fn = run_ctx
                .unpack_map
                .peek(&call.task_id, &Guard::new())
                .cloned();

            let out_vals = if let Some(unpack_fn) = unpack_fn {
                (unpack_fn)(res)
            } else {
                vec![res]
            };

            let child_ctx = run_ctx.store.create_child(event.ctx_id);
            run_ctx.active_ctxs.fetch_add(1, Ordering::Release);

            if out_vals.len() == 1 {
                run_ctx
                    .store
                    .set_value(child_ctx, call.outputs[0], out_vals[0].clone())
                    .await?;
            } else {
                for (out_id, val) in call.outputs.iter().copied().zip(out_vals.into_iter()) {
                    run_ctx.store.set_value(child_ctx, out_id, val).await?;
                }
            }

            if let Some(next_op) = run_ctx
                .kernel
                .resolve_next(run_ctx.store, child_ctx, &operation.next)
                .await?
            {
                drive_from(run_ctx, child_ctx, next_op, Some(origin_op_id)).await?;
            } else {
                // Return terminator.
                let return_var = match &operation.next {
                    namu_core::ir::Next::Return { var } => *var,
                    _ => None,
                };
                if let Some(var) = return_var {
                    let val = run_ctx.store.get_value(child_ctx, var).await?;
                    let _ = run_ctx.result_tx.send(val);
                } else {
                    let _ = run_ctx.result_tx.send(Value::new(()));
                }
                finish_ctx(run_ctx, child_ctx);
            }
        }
        Err(err) => {
            if !err.is::<namu_core::TaskEnd>() {
                eprintln!("[dispatcher::{}] error: {err}", event.task_name);
            }
            finish_ctx(run_ctx, event.ctx_id);
        }
    }
    Ok(())
}

fn finish_ctx(run_ctx: &RunContext<'_>, ctx_id: ContextId) {
    if run_ctx.finished_ctxs.get(&ctx_id).is_some() {
        return;
    }
    let _ = run_ctx.finished_ctxs.insert(ctx_id, ());
    run_ctx.store.finish_context(ctx_id);
    if run_ctx.active_ctxs.fetch_sub(1, Ordering::AcqRel) == 1 {
        let _ = run_ctx.finish_tx.send(());
    }
}
