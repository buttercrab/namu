//! Executes work items: drives contexts through the workflow IR on this worker, runs the tasks
//! it can run, and hands everything else to other workers.
//!
//! Processing a work item never changes shared state directly. It drives the context locally
//! until one terminal action (a handoff, fan-out, collect, return, or failure) and returns the
//! resulting [`Effects`], which are committed atomically with the ack (see `commit`). A worker
//! that crashes mid-item therefore leaves no trace: the item is redelivered and recomputed from
//! its context, which is durable.
//!
//! Branch accounting (fan-in termination) lives in the commit script:
//! - A fan-out over `n` items replaces its branch with `n` child branches in a new scope whose
//!   `live` counter starts at `n`. The scope id is the id of the context that fanned out.
//! - A branch that ends (returns, fails, or arrives at a collect) decrements its innermost scope.
//!   The branch that brings it to zero schedules the collect continuation (a `CollectReady` item)
//!   if any branch reached the collect. Otherwise the scope's own branch has ended too, and the
//!   decrement moves one scope outwards.
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

use crate::commit::{Delivery, Effects, commit};
use crate::runtime::{Runtimes, encode_input};
use crate::tree::{IdMinter, RunTree, TreeStore, Writer};

/// Where a context goes next.
struct Step {
    ctx: u64,
    op: usize,
    pred: Option<usize>,
}

/// How a step of local driving ends.
enum Outcome {
    /// Keep driving locally.
    Step(Step),
    /// The work item is finished; these are its effects.
    Done(Effects),
}

/// Deliveries beyond this many fail the branch instead of retrying (poison items).
pub const MAX_ATTEMPTS: usize = 3;

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

    /// Processes one delivery and commits its effects. An error leaves the delivery pending, so
    /// it is retried (by any worker) once its lease expires.
    pub async fn handle(&self, item: WorkItem, delivery: &Delivery) -> anyhow::Result<()> {
        tracing::debug!(?item, ?delivery, "handle");
        let run = self.run(item.run_id()).await?;
        let effects = if delivery.attempts > MAX_ATTEMPTS {
            self.give_up(&run, &item, delivery).await?
        } else {
            self.process(&run, &item).await?
        };
        // Every node the effects can expose must be readable before they are committed.
        self.writer.flush().await;
        let mut redis = self.redis.clone();
        if !commit(&mut redis, run.id, delivery, &effects, now_ms()).await? {
            tracing::info!(?delivery, "already committed by an earlier attempt");
        }
        Ok(())
    }

    async fn process(&self, run: &Run, item: &WorkItem) -> anyhow::Result<Effects> {
        let outcome = match item {
            WorkItem::Continue { ctx, op, pred, .. } => Outcome::Step(Step {
                ctx: *ctx,
                op: *op,
                pred: *pred,
            }),
            WorkItem::Call { ctx, op, .. } => self.call(run, *ctx, *op).await?,
            WorkItem::CollectReady { scope, op, .. } => {
                self.collect_ready(run, *scope, *op).await?
            }
        };
        match outcome {
            Outcome::Step(step) => self.drive(run, step).await,
            Outcome::Done(effects) => Ok(effects),
        }
    }

    /// A delivery that keeps failing: fail its branch so the run can finish.
    async fn give_up(
        &self,
        run: &Run,
        item: &WorkItem,
        delivery: &Delivery,
    ) -> anyhow::Result<Effects> {
        let mut effects = Effects::default();
        effects.failure(format!(
            "gave up after {} attempts: {item:?}",
            delivery.attempts - 1
        ));
        let scopes = match item {
            WorkItem::Continue { ctx, .. } | WorkItem::Call { ctx, .. } => {
                run.tree.node(*ctx).await?.scopes.clone()
            }
            // The continuation never ran: the branch that fanned out ends.
            WorkItem::CollectReady { scope, .. } => run.tree.node(*scope).await?.scopes.clone(),
        };
        effects.end_branch(&scopes);
        Ok(effects)
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

    /// Drives a context locally until a terminal action.
    async fn drive(&self, run: &Run, mut step: Step) -> anyhow::Result<Effects> {
        let store = TreeStore(&run.tree);
        loop {
            let plan = self
                .kernel
                .drive_until_action(&run.workflow, &store, step.ctx as usize, step.op, step.pred)
                .await?;
            let outcome = match plan {
                KernelPlan::Dispatch {
                    op_id,
                    ctx_id,
                    call,
                    ..
                } => {
                    let key = run.manifest(&call.task_id)?.runtime_key();
                    if self.runtime_keys.contains(&key) {
                        self.call(run, ctx_id as u64, op_id).await?
                    } else {
                        // Hand off to a worker that has this runtime.
                        let mut effects = Effects::default();
                        effects.enqueue(
                            &keys::queue_runtime(&key),
                            &WorkItem::Call {
                                run_id: run.id,
                                ctx: ctx_id as u64,
                                op: op_id,
                            },
                        )?;
                        Outcome::Done(effects)
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
                    Outcome::Done(
                        self.fan_out(run, ctx_id as u64, op_id, fan_out.output, items)
                            .await?,
                    )
                }
                KernelPlan::Collect {
                    op_id,
                    ctx_id,
                    collect,
                } => {
                    let ctx = ctx_id as u64;
                    let node = run.tree.node(ctx).await?;
                    let scope = *node
                        .scopes
                        .last()
                        .context("collect outside of any fan-out")?;
                    let item = run.tree.get_value(ctx, collect.input).await?;
                    let mut effects = Effects::default();
                    effects.collect(scope, op_id, &item)?;
                    effects.end_branch(&node.scopes);
                    Outcome::Done(effects)
                }
                KernelPlan::Return { ctx_id, return_var } => {
                    Outcome::Done(self.finish_return(run, ctx_id as u64, return_var).await?)
                }
            };
            match outcome {
                Outcome::Step(next) => step = next,
                Outcome::Done(effects) => return Ok(effects),
            }
        }
    }

    /// Executes the call at `op_id` (whose inputs are visible from `ctx`).
    async fn call(&self, run: &Run, ctx: u64, op_id: usize) -> anyhow::Result<Outcome> {
        let operation = &run.workflow.operations[op_id];
        let call = operation.call.as_ref().context("operation has no call")?;
        let manifest = run.manifest(&call.task_id)?;
        let store = TreeStore(&run.tree);
        let inputs = store.get_values(ctx as usize, &call.inputs).await?;
        let input = encode_input(manifest.input_arity, inputs)?;

        let runner = self.runtimes.runner(manifest).await?;
        // Task calls are CPU-bound; run them on this worker thread instead of hopping to the
        // blocking pool (the hop alone costs microseconds per step).
        let output = tokio::task::block_in_place(|| runner.call(&input))?;
        self.writer
            .hincr(keys::run_calls(run.id), self.worker_id.clone(), 1);

        let output = match output {
            Ok(output) => output,
            Err(task_error) => {
                // The task itself failed: deterministic, so the branch fails without retrying.
                let mut effects = Effects::default();
                effects.failure(format!("{}: {task_error}", call.task_id));
                effects.end_branch(&run.tree.node(ctx).await?.scopes);
                return Ok(Outcome::Done(effects));
            }
        };
        let output: JsonValue = serde_json::from_str(&output).context("task output is not JSON")?;

        if manifest.task_kind == TaskKind::Stream {
            // A stream is a list whose items each continue in their own branch.
            let JsonValue::Array(items) = output else {
                anyhow::bail!("stream task {} did not return a list", call.task_id);
            };
            let output_id = *call.outputs.first().context("stream task has no output")?;
            return Ok(Outcome::Done(
                self.fan_out(run, ctx, op_id, output_id, items).await?,
            ));
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
        let outcome = self.advance(run, child, op_id, &operation.next).await?;
        if manifest.checkpoint
            && let Outcome::Step(step) = outcome
        {
            // Commit the output now: recovery resumes after this task instead of re-running it.
            let mut effects = Effects::default();
            effects.enqueue(
                keys::QUEUE_ANY,
                &WorkItem::Continue {
                    run_id: run.id,
                    ctx: step.ctx,
                    op: step.op,
                    pred: step.pred,
                },
            )?;
            return Ok(Outcome::Done(effects));
        }
        Ok(outcome)
    }

    /// Builds the collected list for a finished scope and continues after the collect.
    async fn collect_ready(&self, run: &Run, scope: u64, op: usize) -> anyhow::Result<Outcome> {
        let mut redis = self.redis.clone();
        let raw: Vec<String> = redis
            .lrange(keys::scope_items(run.id, scope), 0, -1)
            .await?;
        let items = raw
            .iter()
            .map(|item| serde_json::from_str(item))
            .collect::<Result<Vec<JsonValue>, _>>()?;
        let operation = &run.workflow.operations[op];
        let collect = operation.collect.as_ref().context("op is not a collect")?;
        // The continuation hangs off the scope's context: per-item values are out of scope.
        let cont = run
            .tree
            .create_child(scope, vec![(collect.output, JsonValue::Array(items))])
            .await?;
        self.advance(run, cont, op, &operation.next).await
    }

    /// Follows `next` from `op_id`, handling a return terminator in place.
    async fn advance(
        &self,
        run: &Run,
        ctx: u64,
        op_id: usize,
        next: &Next,
    ) -> anyhow::Result<Outcome> {
        let store = TreeStore(&run.tree);
        match self.kernel.resolve_next(&store, ctx as usize, next).await? {
            Some(op) => Ok(Outcome::Step(Step {
                ctx,
                op,
                pred: Some(op_id),
            })),
            None => {
                let var = match next {
                    Next::Return { var } => *var,
                    _ => None,
                };
                Ok(Outcome::Done(self.finish_return(run, ctx, var).await?))
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
    ) -> anyhow::Result<Effects> {
        let parent = run.tree.node(ctx).await?;
        let mut effects = Effects::default();
        if items.is_empty() {
            effects.end_branch(&parent.scopes);
            return Ok(effects);
        }
        effects.set_scope(ctx, items.len());
        let mut scopes = parent.scopes.clone();
        scopes.push(ctx);
        let next = &run.workflow.operations[op_id].next;
        for item in items {
            let child = run.tree.create(NodeRecord {
                parent: Some(ctx),
                scopes: scopes.clone(),
                values: vec![(output_id, item)],
            })?;
            match self.advance(run, child, op_id, next).await? {
                Outcome::Step(step) => effects.enqueue(
                    keys::QUEUE_ANY,
                    &WorkItem::Continue {
                        run_id: run.id,
                        ctx: step.ctx,
                        op: step.op,
                        pred: step.pred,
                    },
                )?,
                Outcome::Done(child_effects) => effects.merge(child_effects),
            }
        }
        Ok(effects)
    }

    async fn finish_return(
        &self,
        run: &Run,
        ctx: u64,
        var: Option<usize>,
    ) -> anyhow::Result<Effects> {
        let mut effects = Effects::default();
        if let Some(var) = var {
            effects.result(&run.tree.get_value(ctx, var).await?)?;
        }
        effects.end_branch(&run.tree.node(ctx).await?.scopes);
        Ok(effects)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
