//! Driving branches through the IR on this worker.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Context;
use namu_core::OpId;
use namu_core::ir::Call;
use namu_core::value::Value;
use tokio::runtime::{Handle, RuntimeFlavor};

use super::delivery::Release;
use super::view::View;
use super::{Inner, Run};
use crate::ids::{NodeId, WorkerId, owner};
use crate::kernel::{self, Plan, Point, Start, ValueStore};
use crate::log::Record;
use crate::message::{Branch, BranchId, Handoff, Report, Reported, Resume};
use crate::scope::{Continuation, Scope, Then};
use crate::tasks::{Args, CallKey, CallTask, Kind, Task, TaskRef, unpack_output};
use crate::tree::Node;

impl Inner {
    pub(super) fn on_handoff(self: &Arc<Self>, handoff: Handoff) {
        let (run, owner) = (handoff.run, owner(handoff.branch.scopes[0]));
        let Some((state, handoff)) = self.known_or_wait(run, owner, handoff) else {
            return;
        };
        let Handoff {
            branch,
            nodes,
            delivery,
            ..
        } = handoff;
        let nodes = state.nodes.receive(nodes);
        let ctx = nodes.iter().find(|node| node.id() == branch.ctx).cloned();
        let durable = self.log.append(vec![
            Record::Nodes { run, nodes },
            Record::Branch {
                run,
                holder: self.id,
                branch: branch.clone(),
            },
        ]);
        self.ack_when(durable, delivery);
        match ctx {
            Some(ctx) => self.spawn_drive(state, branch, ctx),
            None => self.spawn_drive_fetching(state, branch),
        }
    }

    /// Drives `branch` from `ctx`, its context, which is held from now on.
    pub(super) fn spawn_drive(self: &Arc<Self>, run: Arc<Run>, branch: Branch, ctx: Arc<Node>) {
        debug_assert_eq!(branch.ctx, ctx.id());
        let at = Driving {
            since: self.ids.peek(),
            ctx,
            branch,
        };
        // Counted from now, not when the task starts, so work placed right after sees it.
        let busy = self.busy();
        let worker = self.clone();
        tokio::spawn(async move {
            worker.drive(&run, at).await;
            drop(busy);
        });
    }

    /// Drives `branch` once its context is fetched: one that is durable, or held elsewhere.
    pub(super) fn spawn_drive_fetching(self: &Arc<Self>, run: Arc<Run>, branch: Branch) {
        let worker = self.clone();
        tokio::spawn(async move {
            match worker.node(&run, branch.ctx).await {
                Ok(ctx) => worker.spawn_drive(run, branch, ctx),
                Err(err) => worker.report(&run, &branch, Report::Failure(format!("{err:#}"))),
            }
        });
    }

    /// Advances `branch` until it ends or leaves this worker. A branch that cannot go on fails,
    /// and its failure is reported like any other end.
    async fn drive(self: &Arc<Self>, run: &Arc<Run>, mut at: Driving) {
        if let Err(err) = self.advance(run, &mut at).await {
            self.report(run, &at.branch, Report::Failure(format!("{err:#}")));
        }
    }

    /// Returns once the branch has been passed on; an error means it has not.
    async fn advance(self: &Arc<Self>, run: &Arc<Run>, at: &mut Driving) -> anyhow::Result<()> {
        let workflow = run.spec.workflow();
        let view = View { worker: self, run };
        loop {
            let plan = match at.branch.resume {
                Resume::Kernel(start) => {
                    kernel::drive(workflow, &run.spec.literals, &view, at.ctx.clone(), start)
                        .await?
                }
                Resume::Call { op } => Plan::Call {
                    op,
                    call: kernel::operation(workflow, op)?
                        .call()
                        .context("resuming at a call without one")?,
                    ctx: at.ctx.clone(),
                },
            };
            match plan {
                Plan::Call { op, call, ctx } => {
                    at.set_ctx(ctx);
                    let task_ref = run.spec.task(op).context("op has no call")?;
                    // A draining worker runs nothing more: the branch moves on.
                    let served = self.tasks.task(task_ref).filter(|_| !self.is_draining());
                    let Some(task) = served else {
                        let to = self.routes.wait_for_server(task_ref, self.skip()).await;
                        at.branch.resume = Resume::Call { op };
                        self.move_branch(run, at, to);
                        return Ok(());
                    };
                    if let Kind::Stream(stream) = &task.kind
                        && let Some(fan_out) = run.spec.fused[op]
                    {
                        self.start_stream(run, at, op, fan_out, stream.clone());
                        return Ok(());
                    }
                    if let Some(group) = &run.spec.parallel[op]
                        && let Some(calls) = self.servable(run, group)?
                    {
                        let last = *group.last().expect("a group has calls");
                        match self.run_group(&view, calls, at).await? {
                            Ok(next) => at.set_ctx(next),
                            Err(failure) => {
                                self.report(run, &at.branch, Report::Failure(failure));
                                return Ok(());
                            }
                        }
                        at.branch.resume = Resume::Kernel(Start::After { op: last });
                        continue;
                    }
                    match self.run_call(&view, task_ref, &task, op, call, at).await? {
                        Ok(next) => {
                            at.set_ctx(next);
                            at.branch.resume = Resume::Kernel(Start::After { op });
                        }
                        // An attempted call: its error is a value, and control takes the error
                        // edge.
                        Err(message) if let Some(catch) = &call.catch => {
                            let error = (catch.error, Value::new(message));
                            at.set_ctx(view.put(&at.ctx, vec![error], Point::Action(op)).await?);
                            at.branch.resume = Resume::Kernel(Start::At {
                                op: catch.next,
                                pred: Some(op),
                            });
                        }
                        Err(message) => {
                            let failure = format!("{}: {message}", task_ref.id);
                            self.report(run, &at.branch, Report::Failure(failure));
                            return Ok(());
                        }
                    }
                    if task.checkpoint {
                        self.checkpoint(run, at).await?;
                    }
                }
                Plan::FanOut { op, fan_out, ctx } => {
                    at.set_ctx(ctx);
                    let list = view.get(&at.ctx, fan_out.input).await?;
                    let items = list.items().context("fan-out")?;
                    self.fan_out(run, at, op, fan_out.output, items);
                    return Ok(());
                }
                Plan::Collect { collect, ctx, .. } => {
                    let item = view.get(&ctx, collect.input).await?;
                    self.end_branch(run, &at.branch, Some(item));
                    return Ok(());
                }
                Plan::Return { ctx, value } => {
                    match value {
                        Some(value) => {
                            let value = view.get(&ctx, value).await?;
                            self.report(run, &at.branch, Report::Result(value));
                        }
                        None => self.end_branch(run, &at.branch, None),
                    }
                    return Ok(());
                }
            }
        }
    }

    /// Runs a task call here and stores its outputs in a new node under the branch's context.
    /// `Ok(Err(_))` is the task's own failure, with its message.
    async fn run_call(
        self: &Arc<Self>,
        view: &View<'_>,
        task_ref: &TaskRef,
        task: &Task,
        op: OpId,
        call: &Call,
        at: &mut Driving,
    ) -> anyhow::Result<Result<Arc<Node>, String>> {
        let args = gather(view, call, &at.ctx).await?;
        let key = at.key(view.run, op);
        let output = self.invoke(task_ref, task, args, key).await?;
        view.run.calls.fetch_add(1, Ordering::Relaxed);
        match output {
            Ok(output) => Ok(Ok(store(view, op, call, task_ref, &at.ctx, output).await?)),
            Err(message) => Ok(Err(message)),
        }
    }

    /// The calls of `group` with the tasks that run them, if this worker serves them all and none
    /// is a checkpoint (whose branch must be recorded right after it). Otherwise the group runs
    /// one call at a time, as any other.
    fn servable<'w>(
        &self,
        run: &'w Run,
        group: &[OpId],
    ) -> anyhow::Result<Option<Vec<Planned<'w>>>> {
        let mut calls = Vec::with_capacity(group.len());
        for &op in group {
            let call = kernel::operation(run.spec.workflow(), op)?
                .call()
                .context("a group holds calls")?;
            let task_ref = run.spec.task(op).context("op has no call")?;
            match self.tasks.task(task_ref) {
                Some(task) if !task.checkpoint => calls.push(Planned {
                    op,
                    call,
                    task_ref,
                    task,
                }),
                _ => return Ok(None),
            }
        }
        Ok(Some(calls))
    }

    /// Runs a group of independent calls at once, and stores their outputs in order.
    async fn run_group(
        self: &Arc<Self>,
        view: &View<'_>,
        calls: Vec<Planned<'_>>,
        at: &mut Driving,
    ) -> anyhow::Result<Result<Arc<Node>, String>> {
        // Every call reads only values from before the group.
        let mut running = Vec::with_capacity(calls.len());
        for planned in &calls {
            let args = gather(view, planned.call, &at.ctx).await?;
            let key = at.key(view.run, planned.op);
            let (worker, task_ref, task) =
                (self.clone(), planned.task_ref.clone(), planned.task.clone());
            running.push(tokio::spawn(async move {
                worker.invoke(&task_ref, &task, args, key).await
            }));
        }
        let mut ctx = at.ctx.clone();
        for (index, planned) in calls.iter().enumerate() {
            let output = (&mut running[index]).await??;
            view.run.calls.fetch_add(1, Ordering::Relaxed);
            let output = match output {
                Ok(output) => output,
                Err(message) => {
                    running[index + 1..].iter().for_each(|call| call.abort());
                    return Ok(Err(format!("{}: {message}", planned.task_ref.id)));
                }
            };
            ctx = store(
                view,
                planned.op,
                planned.call,
                planned.task_ref,
                &ctx,
                output,
            )
            .await?;
        }
        Ok(Ok(ctx))
    }

    /// Runs a task on `args`, whatever its kind, as the call named `key`.
    async fn invoke(
        self: &Arc<Self>,
        task_ref: &TaskRef,
        task: &Task,
        args: Args,
        key: CallKey,
    ) -> anyhow::Result<Result<Value, String>> {
        match &task.kind {
            Kind::Call(task) => self.call(task, args, key).await,
            // Not feeding a fan-out: the stream is read to its end into a list.
            Kind::Stream(stream) => self.read_stream(stream, args, key).await,
            Kind::Batch(batch) => self.batch_call(task_ref, batch, args, key).await,
        }
    }

    async fn call(
        &self,
        task: &Arc<dyn CallTask>,
        args: Args,
        key: CallKey,
    ) -> anyhow::Result<Result<Value, String>> {
        let _slot = self.task_slots.acquire().await?;
        run_task(|| task.call(args, key))
    }

    /// Records the branch past a checkpoint task, and waits until that is durable, so recovery
    /// never runs the task again.
    async fn checkpoint(&self, run: &Run, at: &mut Driving) -> anyhow::Result<()> {
        let left = self.gone(run, &at.branch);
        at.branch.hop += 1;
        let durable = self.log.append(vec![
            Record::Nodes {
                run: run.id(),
                nodes: self.trail(at),
            },
            Record::Branch {
                run: run.id(),
                holder: self.id,
                branch: at.branch.clone(),
            },
        ]);
        durable.await.context("recording a checkpoint failed")?;
        self.log.record(vec![left]);
        at.since = self.ids.peek();
        Ok(())
    }

    /// Opens a scope with one branch per item. This worker owns the scope; the branch that fanned
    /// out waits in it until every item's branch has ended (at once, if there are none).
    fn fan_out(
        self: &Arc<Self>,
        run: &Arc<Run>,
        at: &Driving,
        op: OpId,
        output: namu_core::ValueId,
        items: Vec<Value>,
    ) {
        let branch = &at.branch;
        let scope = self.ids.next();
        let continuation = Continuation {
            branch: branch.id,
            hop: branch.hop,
            calls: branch.calls,
            ctx: branch.ctx,
            scopes: branch.scopes.clone(),
            collect: run.spec.collect_ops[op],
        };
        let trail = self.trail(at);
        let count = items.len() as u64;
        let then = Then::Continue {
            continuation: continuation.clone(),
            ctx: at.ctx.clone(),
        };
        // The opening branch's record goes once the scope and every child are durable elsewhere.
        let release = self.release_after(items.len() + 1, self.gone(run, branch));
        let durable = self.log.append(vec![
            Record::Nodes {
                run: run.id(),
                nodes: trail.clone(),
            },
            Record::Scope {
                run: run.id(),
                scope,
                continuation: Some(continuation),
                source: None,
                count: Some(count),
            },
        ]);
        self.release_when(durable, release.clone());
        if items.is_empty() {
            return self.scope_over(run, scope, Scope::new(then, Some(0)));
        }
        run.open_scope(scope, Scope::new(then, Some(count)));

        let inner: Arc<[_]> = branch.scopes.iter().copied().chain([scope]).collect();
        let live = run.spec.liveness.after_action(op);
        for (index, item) in (0..).zip(items) {
            let child = self.grow(run, &at.ctx, live, vec![(output, item)]);
            let ctx = child.id();
            let nodes = [child].into_iter().chain(trail.iter().cloned()).collect();
            let child = Branch {
                id: BranchId { scope, index },
                hop: 0,
                calls: 0,
                ctx,
                resume: Resume::Kernel(Start::After { op }),
                scopes: inner.clone(),
            };
            let to = self.destination();
            self.hand_off(run, to, child, nodes, Some(release.clone()));
        }
    }
}

/// Runs task code. Tasks are CPU-bound; they run on this thread rather than hopping to the
/// blocking pool.
pub(super) fn run_task<T>(f: impl FnOnce() -> T) -> T {
    match Handle::current().runtime_flavor() {
        RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// A call of a group, with the task that runs it here.
struct Planned<'w> {
    op: OpId,
    call: &'w Call,
    task_ref: &'w TaskRef,
    task: Task,
}

/// A call's arguments, read from `ctx`.
async fn gather(view: &View<'_>, call: &Call, ctx: &Arc<Node>) -> anyhow::Result<Args> {
    let mut args = Vec::with_capacity(call.inputs.len());
    for &input in &call.inputs {
        args.push(view.get(ctx, input).await?);
    }
    Ok(args)
}

/// Stores a call's outputs in a node under `ctx`, and returns it.
async fn store(
    view: &View<'_>,
    op: OpId,
    call: &Call,
    task_ref: &TaskRef,
    ctx: &Arc<Node>,
    output: Value,
) -> anyhow::Result<Arc<Node>> {
    let outputs = unpack_output(output, call.outputs.len())
        .with_context(|| format!("task {}", task_ref.id))?;
    if outputs.is_empty() {
        return Ok(ctx.clone());
    }
    let values = call.outputs.iter().copied().zip(outputs).collect();
    view.put(ctx, values, Point::Action(op)).await
}

/// A branch being driven on this worker.
pub(super) struct Driving {
    /// The branch; its `ctx` is the id of `ctx`.
    pub branch: Branch,
    /// The branch's context, held while it is driven.
    pub ctx: Arc<Node>,
    /// The first id minted after the branch arrived (or passed a checkpoint): nodes of its chain
    /// with ids from here on were created here for it, and are not durable yet.
    pub since: NodeId,
}

impl Driving {
    fn set_ctx(&mut self, ctx: Arc<Node>) {
        self.branch.ctx = ctx.id();
        self.ctx = ctx;
    }

    /// The key of the branch's next call, at `op`, which it counts.
    pub(super) fn key(&mut self, run: &Run, op: OpId) -> CallKey {
        let key = CallKey {
            run: run.id(),
            branch: self.branch.id,
            call: self.branch.calls,
            op,
        };
        self.branch.calls += 1;
        key
    }
}

impl Inner {
    /// The nodes of the branch's chain created here since it arrived, innermost first, and the
    /// node they hang from if this worker holds it: what a handoff ships and a record logs.
    pub(super) fn trail(&self, at: &Driving) -> Vec<Arc<Node>> {
        let mut nodes = Vec::new();
        let mut cursor = Some(&at.ctx);
        while let Some(node) = cursor {
            nodes.push(node.clone());
            let created_here = owner(node.id()) == self.id && node.id() >= at.since;
            cursor = node.parent().filter(|_| created_here);
        }
        nodes
    }

    /// The record that removes the branch's current stop on this worker from the log.
    pub(super) fn gone(&self, run: &Run, branch: &Branch) -> Record {
        Record::BranchGone {
            run: run.id(),
            holder: self.id,
            id: branch.id,
            hop: branch.hop,
        }
    }

    /// Moves the branch at `at` to worker `to`, with the nodes it created here.
    fn move_branch(self: &Arc<Self>, run: &Arc<Run>, at: &Driving, to: WorkerId) {
        let release = self.release_after(1, self.gone(run, &at.branch));
        let branch = Branch {
            hop: at.branch.hop + 1,
            ..at.branch.clone()
        };
        self.hand_off(run, to, branch, self.trail(at), Some(release));
    }

    pub(super) fn hand_off(
        self: &Arc<Self>,
        run: &Arc<Run>,
        to: WorkerId,
        branch: Branch,
        nodes: Vec<Arc<Node>>,
        release: Option<Arc<Release>>,
    ) {
        let run = run.id();
        self.send_kept(
            to,
            |delivery| {
                Handoff {
                    run,
                    branch,
                    nodes,
                    delivery,
                }
                .into()
            },
            release,
        );
    }

    /// The branch ends in its innermost scope.
    pub(super) fn end_branch(self: &Arc<Self>, run: &Run, branch: &Branch, item: Option<Value>) {
        let release = self.release_after(1, self.gone(run, branch));
        self.send_end(run, branch.id, item, Some(release));
    }

    /// The branch reports to the run's owner, which ends it in its scope once the report is
    /// recorded.
    pub(super) fn report(self: &Arc<Self>, run: &Run, branch: &Branch, report: Report) {
        let release = self.release_after(1, self.gone(run, branch));
        self.send_report(run, branch.id, report, Some(release));
    }

    /// Sends `branch`'s report to the run's owner.
    pub(super) fn send_report(
        self: &Arc<Self>,
        run: &Run,
        branch: BranchId,
        report: Report,
        release: Option<Arc<Release>>,
    ) {
        if let Report::Failure(failure) = &report {
            tracing::warn!(run = %run.id(), "branch failed: {failure}");
        }
        let run_id = run.id();
        self.send_kept(
            run.spec.owner(),
            |delivery| {
                Reported {
                    run: run_id,
                    branch,
                    report,
                    delivery,
                }
                .into()
            },
            release,
        );
    }
}
