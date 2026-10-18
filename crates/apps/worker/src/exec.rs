//! Executes work items: drives contexts through the workflow IR on this worker, runs the tasks
//! it can run, and hands everything else to other workers.
//!
//! Branch accounting (fan-in termination):
//! - A fan-out over `n` items replaces its branch with `n` child branches in a new scope whose
//!   `live` counter starts at `n`. The scope id is the id of the context that fanned out.
//! - A branch that ends (returns, fails, or arrives at a collect) decrements its innermost scope.
//!   The branch that brings it to zero fires the collect continuation, if any branch reached the
//!   collect. Otherwise the scope's own branch has ended too, and the decrement moves one scope
//!   outwards.
//! - Branches outside any scope count against the run's `live` counter; the run finishes when that
//!   reaches zero.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use namu_core::ir::{Next, Workflow};
use namu_engine::kernel::{EngineKernel, JsonRuntime, KernelPlan, ValueStore};
use namu_proto::{NodeRecord, RunMeta, TaskKind, TaskManifest, WorkItem, keys};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde_json::Value as JsonValue;
use uuid::Uuid;

use crate::runtime::{Runtimes, encode_input};
use crate::tree::{IdMinter, RunTree, TreeStore, Writer};

/// Where a context goes next.
struct Step {
    ctx: u64,
    op: usize,
    pred: Option<usize>,
}

pub struct Run {
    pub id: Uuid,
    workflow: Workflow,
    manifests: HashMap<String, TaskManifest>,
    tree: RunTree,
}

impl Run {
    fn manifest(&self, task_id: &str) -> anyhow::Result<&TaskManifest> {
        self.manifests
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("no manifest for task {task_id}"))
    }
}

pub struct Executor {
    pub worker_id: String,
    runtime_keys: HashSet<String>,
    redis: ConnectionManager,
    writer: Writer,
    ids: Arc<IdMinter>,
    runtimes: Runtimes,
    kernel: EngineKernel<JsonRuntime>,
    runs: tokio::sync::Mutex<HashMap<Uuid, Arc<Run>>>,
}

impl Executor {
    pub fn new(
        worker_id: String,
        runtime_keys: HashSet<String>,
        redis: ConnectionManager,
        writer: Writer,
        ids: Arc<IdMinter>,
        runtimes: Runtimes,
    ) -> Self {
        Self {
            worker_id,
            runtime_keys,
            redis,
            writer,
            ids,
            runtimes,
            kernel: EngineKernel::new(JsonRuntime),
            runs: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    pub async fn handle(&self, item: WorkItem) -> anyhow::Result<()> {
        tracing::debug!(?item, "handle");
        let run = self.run(item.run_id()).await?;
        let result = match &item {
            WorkItem::Continue { ctx, op, pred, .. } => {
                self.drive(
                    &run,
                    Step {
                        ctx: *ctx,
                        op: *op,
                        pred: *pred,
                    },
                )
                .await
            }
            WorkItem::Call { ctx, op, .. } => match self.call(&run, *ctx, *op).await {
                Ok(Some(step)) => self.drive(&run, step).await,
                Ok(None) => Ok(()),
                Err(err) => Err(err),
            },
        };
        if let Err(err) = result {
            // An infrastructure error: record it and end the branch so the run still finishes.
            let ctx = match item {
                WorkItem::Continue { ctx, .. } | WorkItem::Call { ctx, .. } => ctx,
            };
            self.fail_branch(&run, ctx, format!("{err:#}")).await?;
        }
        Ok(())
    }

    async fn run(&self, run_id: Uuid) -> anyhow::Result<Arc<Run>> {
        let mut runs = self.runs.lock().await;
        if let Some(run) = runs.get(&run_id) {
            return Ok(run.clone());
        }
        let mut redis = self.redis.clone();
        let raw: Option<String> = redis.get(keys::run_meta(run_id)).await?;
        let meta: RunMeta = serde_json::from_str(&raw.context("unknown run")?)?;
        let workflow: Workflow = serde_json::from_value(meta.ir)?;
        let mut manifests = HashMap::new();
        for (task_id, version) in &meta.task_versions {
            let raw: Option<String> = redis.get(keys::task(task_id, version)).await?;
            let raw = raw.with_context(|| format!("task {task_id}@{version} not published"))?;
            manifests.insert(task_id.clone(), serde_json::from_str(&raw)?);
        }
        let run = Arc::new(Run {
            id: run_id,
            workflow,
            manifests,
            tree: RunTree::new(
                run_id,
                self.worker_id.clone(),
                self.redis.clone(),
                self.writer.clone(),
                self.ids.clone(),
            ),
        });
        runs.insert(run_id, run.clone());
        Ok(run)
    }

    /// Drives a context locally for as long as possible.
    async fn drive(&self, run: &Run, mut step: Step) -> anyhow::Result<()> {
        let store = TreeStore(&run.tree);
        loop {
            let plan = self
                .kernel
                .drive_until_action(&run.workflow, &store, step.ctx as usize, step.op, step.pred)
                .await?;
            let next = match plan {
                KernelPlan::Dispatch {
                    op_id,
                    ctx_id,
                    call,
                    ..
                } => {
                    let manifest = run.manifest(&call.task_id)?;
                    let key = manifest.runtime_key();
                    if self.runtime_keys.contains(&key) {
                        self.call(run, ctx_id as u64, op_id).await?
                    } else {
                        // Hand off to a worker that has this runtime.
                        let item = WorkItem::Call {
                            run_id: run.id,
                            ctx: ctx_id as u64,
                            op: op_id,
                        };
                        self.enqueue(&keys::queue_runtime(&key), &[item]).await?;
                        None
                    }
                }
                KernelPlan::FanOut {
                    op_id,
                    ctx_id,
                    fan_out,
                } => {
                    let list = run.tree.get_value(ctx_id as u64, fan_out.input).await?;
                    let JsonValue::Array(items) = list else {
                        anyhow::bail!("fan-out input is not a list");
                    };
                    self.fan_out(run, ctx_id as u64, op_id, fan_out.output, items)
                        .await?
                }
                KernelPlan::Collect {
                    op_id,
                    ctx_id,
                    collect,
                } => {
                    let ctx = ctx_id as u64;
                    tracing::debug!(ctx, op_id, "collect: arrive");
                    let node = run.tree.node(ctx).await?;
                    let scope = *node
                        .scopes
                        .last()
                        .context("collect outside of any fan-out")?;
                    let item = run.tree.get_value(ctx, collect.input).await?;
                    tracing::debug!(ctx, scope, "collect: push");
                    let mut redis = self.redis.clone();
                    let _: () = redis
                        .rpush(
                            keys::scope_items(run.id, scope),
                            serde_json::to_string(&item)?,
                        )
                        .await?;
                    let _: () = redis
                        .hset(keys::scope(run.id, scope), "collect_op", op_id)
                        .await?;
                    self.end_branch(run, ctx).await?
                }
                KernelPlan::Return { ctx_id, return_var } => {
                    self.finish_return(run, ctx_id as u64, return_var).await?
                }
            };
            match next {
                Some(next) => step = next,
                None => return Ok(()),
            }
        }
    }

    /// Executes the call at `op_id` (whose inputs are visible from `ctx`) and returns where the
    /// context continues, if it continues on this worker.
    async fn call(&self, run: &Run, ctx: u64, op_id: usize) -> anyhow::Result<Option<Step>> {
        let operation = &run.workflow.operations[op_id];
        let call = operation.call.as_ref().context("operation has no call")?;
        let manifest = run.manifest(&call.task_id)?;
        let store = TreeStore(&run.tree);
        let inputs = store.get_values(ctx as usize, &call.inputs).await?;
        let input = encode_input(manifest.input_arity, inputs)?;

        tracing::debug!(task = %call.task_id, ctx, "call: loading runner");
        let runner = self.runtimes.runner(manifest).await?;
        tracing::debug!(task = %call.task_id, ctx, "call: running");
        // Task calls are CPU-bound; run them on this worker thread instead of hopping to the
        // blocking pool (the hop alone costs microseconds per step).
        // Task calls are CPU-bound; run them on this worker thread instead of hopping to the
        // blocking pool (the hop alone costs microseconds per step).
        let output = tokio::task::block_in_place(|| runner.call(&input))?;
        self.writer
            .hincr(keys::run_calls(run.id), self.worker_id.clone(), 1);

        let output = match output {
            Ok(output) => output,
            Err(task_error) => {
                self.fail_branch(run, ctx, format!("{}: {task_error}", call.task_id))
                    .await?;
                return Ok(None);
            }
        };
        tracing::debug!(task = %call.task_id, ctx, "call: returned");
        let output: JsonValue = serde_json::from_str(&output).context("task output is not JSON")?;

        if manifest.task_kind == TaskKind::Stream {
            // A stream is a list whose items each continue in their own branch.
            let JsonValue::Array(items) = output else {
                anyhow::bail!("stream task {} did not return a list", call.task_id);
            };
            let output_id = *call.outputs.first().context("stream task has no output")?;
            return self.fan_out(run, ctx, op_id, output_id, items).await;
        }

        let values = match call.outputs.len() {
            0 => vec![],
            1 => vec![(call.outputs[0], output)],
            n => {
                let JsonValue::Array(parts) = output else {
                    anyhow::bail!("task {} must return {n} outputs", call.task_id);
                };
                anyhow::ensure!(
                    parts.len() == n,
                    "task {} output arity mismatch",
                    call.task_id
                );
                call.outputs.iter().copied().zip(parts).collect()
            }
        };
        let child = if values.is_empty() {
            ctx
        } else {
            run.tree.create_child(ctx, values).await?
        };
        tracing::debug!(ctx, child, "call: child created");
        self.advance(run, child, op_id, &operation.next).await
    }

    /// Follows `next` from `op_id`, handling a return terminator in place.
    async fn advance(
        &self,
        run: &Run,
        ctx: u64,
        op_id: usize,
        next: &Next,
    ) -> anyhow::Result<Option<Step>> {
        let store = TreeStore(&run.tree);
        match self.kernel.resolve_next(&store, ctx as usize, next).await? {
            Some(op) => Ok(Some(Step {
                ctx,
                op,
                pred: Some(op_id),
            })),
            None => {
                let var = match next {
                    Next::Return { var } => *var,
                    _ => None,
                };
                self.finish_return(run, ctx, var).await
            }
        }
    }

    /// Splits `ctx` into one child per item and hands the children to any worker.
    async fn fan_out(
        &self,
        run: &Run,
        ctx: u64,
        op_id: usize,
        output_id: usize,
        items: Vec<JsonValue>,
    ) -> anyhow::Result<Option<Step>> {
        if items.is_empty() {
            return self.end_branch(run, ctx).await;
        }
        let mut redis = self.redis.clone();
        let _: () = redis
            .hset(keys::scope(run.id, ctx), "live", items.len())
            .await?;

        let parent = run.tree.node(ctx).await?;
        let mut scopes = parent.scopes.clone();
        scopes.push(ctx);
        let next = &run.workflow.operations[op_id].next;
        let mut work = Vec::with_capacity(items.len());
        for item in items {
            let child = run.tree.create(NodeRecord {
                parent: Some(ctx),
                scopes: scopes.clone(),
                values: vec![(output_id, item)],
            })?;
            if let Some(step) = self.advance(run, child, op_id, next).await? {
                work.push(WorkItem::Continue {
                    run_id: run.id,
                    ctx: step.ctx,
                    op: step.op,
                    pred: step.pred,
                });
            }
        }
        self.enqueue(keys::QUEUE_ANY, &work).await?;
        Ok(None)
    }

    async fn finish_return(
        &self,
        run: &Run,
        ctx: u64,
        var: Option<usize>,
    ) -> anyhow::Result<Option<Step>> {
        if let Some(var) = var {
            let value = run.tree.get_value(ctx, var).await?;
            let mut redis = self.redis.clone();
            let _: () = redis
                .rpush(keys::run_results(run.id), serde_json::to_string(&value)?)
                .await?;
        }
        self.end_branch(run, ctx).await
    }

    async fn fail_branch(&self, run: &Run, ctx: u64, message: String) -> anyhow::Result<()> {
        tracing::warn!(run = %run.id, ctx, "branch failed: {message}");
        let mut redis = self.redis.clone();
        let _: () = redis.rpush(keys::run_failures(run.id), message).await?;
        if let Some(step) = self.end_branch(run, ctx).await? {
            Box::pin(self.drive(run, step)).await?;
        }
        Ok(())
    }

    /// Ends the branch `ctx`. Returns the collect continuation if this was the last branch of a
    /// scope that has one.
    async fn end_branch(&self, run: &Run, ctx: u64) -> anyhow::Result<Option<Step>> {
        // Every write this worker made for the branch must be visible before others can observe
        // the decrement.
        tracing::debug!(ctx, "end_branch: flush");
        self.writer.flush().await;
        tracing::debug!(ctx, "end_branch: flushed");
        let mut redis = self.redis.clone();
        let mut ctx = ctx;
        loop {
            let node = run.tree.node(ctx).await?;
            let Some(&scope) = node.scopes.last() else {
                let live: i64 = redis.hincr(keys::run_state(run.id), "live", -1).await?;
                if live == 0 {
                    self.finish_run(run).await?;
                }
                return Ok(None);
            };
            let live: i64 = redis.hincr(keys::scope(run.id, scope), "live", -1).await?;
            if live > 0 {
                return Ok(None);
            }
            let collect_op: Option<usize> =
                redis.hget(keys::scope(run.id, scope), "collect_op").await?;
            let Some(collect_op) = collect_op else {
                // Every branch of the scope ended without reaching a collect: the branch that
                // fanned out has ended too.
                ctx = scope;
                continue;
            };
            let raw: Vec<String> = redis
                .lrange(keys::scope_items(run.id, scope), 0, -1)
                .await?;
            let items = raw
                .iter()
                .map(|item| serde_json::from_str(item))
                .collect::<Result<Vec<JsonValue>, _>>()?;
            let operation = &run.workflow.operations[collect_op];
            let collect = operation.collect.as_ref().context("op is not a collect")?;
            // The continuation hangs off the scope's context: per-item values are out of scope.
            let cont = run
                .tree
                .create_child(scope, vec![(collect.output, JsonValue::Array(items))])
                .await?;
            return Box::pin(self.advance(run, cont, collect_op, &operation.next)).await;
        }
    }

    async fn finish_run(&self, run: &Run) -> anyhow::Result<()> {
        let mut redis = self.redis.clone();
        let failures: usize = redis.llen(keys::run_failures(run.id)).await?;
        let status = if failures > 0 {
            "partial_failed"
        } else {
            "succeeded"
        };
        let _: () = redis
            .hset_multiple(
                keys::run_state(run.id),
                &[
                    ("status", status.to_string()),
                    ("finished_at_ms", now_ms().to_string()),
                ],
            )
            .await?;
        tracing::info!(run = %run.id, status, "run finished");
        Ok(())
    }

    async fn enqueue(&self, queue: &str, items: &[WorkItem]) -> anyhow::Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        // The receiver may run on another machine: make every node it could read visible first.
        self.writer.flush().await;
        let mut pipe = redis::pipe();
        for item in items {
            pipe.cmd("XADD")
                .arg(queue)
                .arg("*")
                .arg("item")
                .arg(serde_json::to_string(item)?)
                .ignore();
        }
        let mut redis = self.redis.clone();
        pipe.query_async::<()>(&mut redis).await?;
        Ok(())
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
