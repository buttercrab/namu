//! Driving branches through the IR on this worker.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, bail};
use namu_core::OpId;
use namu_core::ir::Call;
use serde_json::Value;
use tokio::runtime::{Handle, RuntimeFlavor};

use super::delivery::Release;
use super::view::View;
use super::{Inner, Run};
use crate::ids::{NodeId, WorkerId, owner};
use crate::kernel::{self, Plan, Point, Start, ValueStore};
use crate::log::Record;
use crate::message::{Branch, BranchId, Message, Report, Resume};
use crate::scope::{Continuation, Scope, Then};
use crate::tasks::{TaskRef, pack_input, unpack_output};
use crate::tree::Node;

impl Inner {
    pub(super) fn on_handoff(self: &Arc<Self>, message: Message) {
        let Message::Handoff { run, branch, .. } = &message else {
            unreachable!()
        };
        let (run, owner) = (*run, owner(branch.scopes[0]));
        let Some((state, message)) = self.known_or_wait(run, owner, message) else {
            return;
        };
        let Message::Handoff {
            branch,
            nodes,
            delivery,
            ..
        } = message
        else {
            unreachable!()
        };
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
        let worker = self.clone();
        tokio::spawn(async move { worker.drive(&run, at).await });
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
                    kernel::drive(workflow, &view, at.ctx.clone(), start).await?
                }
                Resume::Call { op } => Plan::Call {
                    op,
                    ctx: at.ctx.clone(),
                },
            };
            match plan {
                Plan::Call { op, ctx } => {
                    at.set_ctx(ctx);
                    let task = run.spec.task(op).context("op has no call")?;
                    if !self.tasks.serves(task) {
                        let to = self.routes.wait_for_server(task).await;
                        at.branch.resume = Resume::Call { op };
                        self.move_branch(run, at, to);
                        return Ok(());
                    }
                    if let Some(fan_out) = run.spec.fused[op]
                        && self.tasks.is_stream(task)
                    {
                        self.start_stream(run, at, op, fan_out);
                        return Ok(());
                    }
                    let call = kernel::operation(workflow, op)?
                        .call
                        .as_ref()
                        .context("op has no call")?;
                    let next = match self.run_call(&view, task, op, call, &at.ctx).await? {
                        Ok(next) => next,
                        Err(failure) => {
                            self.report(run, &at.branch, Report::Failure(failure));
                            return Ok(());
                        }
                    };
                    at.set_ctx(next);
                    at.branch.resume = Resume::Kernel(Start::After { op });
                    if self.tasks.checkpoint(task) {
                        self.checkpoint(run, at).await?;
                    }
                }
                Plan::FanOut { op, ctx } => {
                    at.set_ctx(ctx);
                    let fan_out = kernel::operation(workflow, op)?
                        .fan_out
                        .as_ref()
                        .context("op has no fan-out")?;
                    let list = view.get(&at.ctx, fan_out.input).await?;
                    let Value::Array(items) = list else {
                        bail!("fan-out over {list}, which is not a list");
                    };
                    self.fan_out(run, at, op, fan_out.output, items);
                    return Ok(());
                }
                Plan::Collect { op, ctx } => {
                    let collect = kernel::operation(workflow, op)?
                        .collect
                        .as_ref()
                        .context("op has no collect")?;
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

    /// Runs a task call here and stores its outputs in a new node under `ctx`. `Ok(Err(_))` is
    /// the task's own failure.
    async fn run_call(
        self: &Arc<Self>,
        view: &View<'_>,
        task: &TaskRef,
        op: OpId,
        call: &Call,
        ctx: &Arc<Node>,
    ) -> anyhow::Result<Result<Arc<Node>, String>> {
        let mut args = Vec::with_capacity(call.inputs.len());
        for &input in &call.inputs {
            args.push(view.get(ctx, input).await?);
        }
        let input = pack_input(args);
        let output = if self.tasks.is_batch(task) {
            self.batch_call(task, input).await?
        } else if self.tasks.is_stream(task) {
            // Not feeding a fan-out: the stream is read to its end into a list.
            self.read_stream(task, input).await?
        } else {
            self.call(task, input).await?
        };
        view.run.calls.fetch_add(1, Ordering::Relaxed);
        let output = match output {
            Ok(output) => output,
            Err(message) => return Ok(Err(format!("{}: {message}", task.id))),
        };
        let outputs = unpack_output(output, call.outputs.len())
            .with_context(|| format!("task {}", task.id))?;
        if outputs.is_empty() {
            return Ok(Ok(ctx.clone()));
        }
        let values = call.outputs.iter().copied().zip(outputs).collect();
        Ok(Ok(view.put(ctx, values, Point::Action(op)).await?))
    }

    async fn call(&self, task: &TaskRef, input: Value) -> anyhow::Result<Result<Value, String>> {
        let _slot = self.task_slots.acquire().await?;
        run_task(|| self.tasks.call(task, input))
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
        let release = Release::new(items.len() + 1, self.gone(run, branch));
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
                ctx,
                resume: Resume::Kernel(Start::After { op }),
                scopes: inner.clone(),
            };
            let to = self.routes.any().unwrap_or(self.id);
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
        let release = Release::new(1, self.gone(run, &at.branch));
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
            |delivery| Message::Handoff {
                run,
                branch,
                nodes,
                delivery,
            },
            release,
        );
    }

    /// The branch ends in its innermost scope.
    pub(super) fn end_branch(self: &Arc<Self>, run: &Run, branch: &Branch, item: Option<Value>) {
        let release = Release::new(1, self.gone(run, branch));
        self.send_end(run, branch.id, item, Some(release));
    }

    /// The branch reports to the run's owner, which ends it in its scope once the report is
    /// recorded.
    pub(super) fn report(self: &Arc<Self>, run: &Run, branch: &Branch, report: Report) {
        let release = Release::new(1, self.gone(run, branch));
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
            |delivery| Message::Report {
                run: run_id,
                branch,
                report,
                delivery,
            },
            release,
        );
    }
}
