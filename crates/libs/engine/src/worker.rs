//! A worker drives branches through the IR. It runs the tasks it serves itself, hands a branch to
//! another worker only when the next task is served elsewhere, owns the scopes it opens, and
//! serves the nodes it created.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, bail, ensure};
use namu_core::OpId;
use namu_core::ir::Call;
use serde_json::Value;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::ids::{Minter, NodeId, RunId, ScopeId, WorkerId, owner};
use crate::kernel::{self, Plan, Start, ValueStore};
use crate::message::{Branch, Message, Report, Resume};
use crate::routes::Routes;
use crate::run::{RunOutcome, RunSpec};
use crate::scope::{Continuation, FULL, Scope, Then, Weight, split};
use crate::tasks::{Tasks, pack_input, unpack_output};
use crate::transport::Transport;
use crate::tree::{Node, Nodes};

#[derive(Clone)]
pub struct Worker {
    inner: Arc<Inner>,
}

struct Inner {
    id: WorkerId,
    ids: Minter,
    tasks: Arc<dyn Tasks>,
    routes: Arc<Routes>,
    transport: Arc<dyn Transport>,
    runs: Mutex<HashMap<RunId, Arc<Run>>>,
    fetches: Mutex<HashMap<u64, oneshot::Sender<Option<Arc<Node>>>>>,
    next_request: AtomicU64,
    /// Bounds concurrent task calls; driving the IR between them is cheap and unbounded.
    task_slots: Semaphore,
}

/// A run as one worker sees it.
struct Run {
    spec: Arc<RunSpec>,
    nodes: Nodes,
    /// The scopes this worker opened and owns.
    scopes: Mutex<HashMap<ScopeId, Scope>>,
}

impl Worker {
    /// Starts a worker that receives its messages from `inbox`.
    pub fn spawn(
        id: WorkerId,
        tasks: Arc<dyn Tasks>,
        routes: Arc<Routes>,
        transport: Arc<dyn Transport>,
        mut inbox: mpsc::UnboundedReceiver<Message>,
    ) -> Self {
        let slots = std::thread::available_parallelism().map_or(4, |n| n.get());
        let inner = Arc::new(Inner {
            id,
            ids: Minter::new(id),
            tasks,
            routes,
            transport,
            runs: Mutex::new(HashMap::new()),
            fetches: Mutex::new(HashMap::new()),
            next_request: AtomicU64::new(0),
            task_slots: Semaphore::new(slots),
        });
        let worker = inner.clone();
        tokio::spawn(async move {
            while let Some(message) = inbox.recv().await {
                worker.receive(message);
            }
        });
        Self { inner }
    }

    pub fn id(&self) -> WorkerId {
        self.inner.id
    }

    pub fn serves(&self, task: &str) -> bool {
        self.inner.tasks.serves(task)
    }

    /// Describes a new run owned by this worker. Install it on every worker, then `start` it.
    pub fn new_run(&self, workflow: namu_core::ir::Workflow) -> anyhow::Result<Arc<RunSpec>> {
        RunSpec::new(workflow, &self.inner.ids)
    }

    pub fn install(&self, spec: Arc<RunSpec>) {
        let run = Arc::new(Run {
            spec,
            nodes: Nodes::default(),
            scopes: Mutex::new(HashMap::new()),
        });
        self.inner.runs.lock().unwrap().insert(run.spec.id, run);
    }

    /// Starts an installed run this worker owns; resolves when the run is over.
    pub fn start(&self, spec: &RunSpec) -> anyhow::Result<oneshot::Receiver<RunOutcome>> {
        ensure!(
            spec.owner() == self.inner.id,
            "run {} is not owned here",
            spec.id
        );
        let run = self.inner.run(spec.id).context("run is not installed")?;
        let (done, outcome) = oneshot::channel();
        run.nodes.insert(
            spec.root,
            Arc::new(Node {
                parent: None,
                values: Vec::new(),
            }),
        );
        run.scopes.lock().unwrap().insert(
            spec.root_scope,
            Scope::new(Then::Finish {
                outcome: RunOutcome::default(),
                done,
            }),
        );
        self.inner.spawn_drive(
            run,
            Branch {
                ctx: spec.root,
                resume: Resume::Kernel(Start::At { op: 0, pred: None }),
                weight: FULL,
                scopes: Arc::from([spec.root_scope]),
            },
        );
        Ok(outcome)
    }
}

impl Inner {
    fn run(&self, id: RunId) -> Option<Arc<Run>> {
        self.runs.lock().unwrap().get(&id).cloned()
    }

    fn receive(self: &Arc<Self>, message: Message) {
        match message {
            Message::Handoff { run, branch, node } => {
                let Some(run) = self.run(run) else {
                    return tracing::error!(%run, "handoff for an unknown run dropped");
                };
                if let Some(node) = node {
                    run.nodes.insert(branch.ctx, node);
                }
                self.spawn_drive(run, branch);
            }
            Message::End {
                run,
                scope,
                weight,
                item,
            } => self.on_end(run, scope, weight, item),
            Message::Report {
                run,
                scope,
                weight,
                report,
            } => self.on_report(run, scope, weight, report),
            Message::Fetch {
                run,
                node,
                request,
                reply_to,
            } => {
                let node = self.run(run).and_then(|run| run.nodes.get(node));
                self.transport
                    .send(reply_to, Message::Fetched { request, node });
            }
            Message::Fetched { request, node } => {
                if let Some(waiter) = self.fetches.lock().unwrap().remove(&request) {
                    let _ = waiter.send(node);
                }
            }
            Message::Forget { run } => {
                self.runs.lock().unwrap().remove(&run);
            }
        }
    }

    fn spawn_drive(self: &Arc<Self>, run: Arc<Run>, branch: Branch) {
        let worker = self.clone();
        tokio::spawn(async move {
            let (scope, weight) = (branch.scope(), branch.weight);
            if let Err(err) = worker.drive(&run, branch).await {
                worker.report(&run, scope, weight, Report::Failure(format!("{err:#}")));
            }
        });
    }

    /// Advances `branch` until it ends or leaves this worker. An error means the branch's weight
    /// has not been passed on, and the branch fails.
    async fn drive(self: &Arc<Self>, run: &Arc<Run>, branch: Branch) -> anyhow::Result<()> {
        let Branch {
            mut ctx,
            mut resume,
            weight,
            scopes,
        } = branch;
        let scope = *scopes.last().context("branch outside of the root scope")?;
        let workflow = &run.spec.workflow;
        let view = View { worker: self, run };
        loop {
            let plan = match resume {
                Resume::Kernel(start) => kernel::drive(workflow, &view, ctx, start).await?,
                Resume::Call { op } => Plan::Call { op, ctx },
            };
            match plan {
                Plan::Call { op, ctx: at } => {
                    let call = kernel::operation(workflow, op)?
                        .call
                        .as_ref()
                        .context("op has no call")?;
                    if !self.tasks.serves(&call.task_id) {
                        let to = self
                            .routes
                            .server(&call.task_id)
                            .with_context(|| format!("no worker serves task {}", call.task_id))?;
                        let resume = Resume::Call { op };
                        let branch = Branch {
                            ctx: at,
                            resume,
                            weight,
                            scopes,
                        };
                        self.hand_off(run, to, branch);
                        return Ok(());
                    }
                    match self.run_call(&view, call, at).await? {
                        Ok(next) => {
                            ctx = next;
                            resume = Resume::Kernel(Start::After { op });
                        }
                        Err(failure) => {
                            self.report(run, scope, weight, Report::Failure(failure));
                            return Ok(());
                        }
                    }
                }
                Plan::FanOut { op, ctx } => {
                    return self.fan_out(run, op, ctx, weight, scopes).await;
                }
                Plan::Collect { op, ctx } => {
                    let collect = kernel::operation(workflow, op)?
                        .collect
                        .as_ref()
                        .context("op has no collect")?;
                    let item = view.get(ctx, collect.input).await?;
                    self.end(run, scope, weight, Some(item));
                    return Ok(());
                }
                Plan::Return { ctx, value } => {
                    match value {
                        Some(value) => {
                            let value = view.get(ctx, value).await?;
                            self.report(run, scope, weight, Report::Result(value));
                        }
                        None => self.end(run, scope, weight, None),
                    }
                    return Ok(());
                }
            }
        }
    }

    /// Runs a task call here and stores its outputs in a new node under `ctx`. `Ok(Err(_))` is
    /// the task's own failure.
    async fn run_call(
        &self,
        view: &View<'_>,
        call: &Call,
        ctx: NodeId,
    ) -> anyhow::Result<Result<NodeId, String>> {
        let mut args = Vec::with_capacity(call.inputs.len());
        for &input in &call.inputs {
            args.push(view.get(ctx, input).await?);
        }
        let output = match self.call(&call.task_id, pack_input(args)).await? {
            Ok(output) => output,
            Err(message) => return Ok(Err(format!("{}: {message}", call.task_id))),
        };
        let outputs = unpack_output(output, call.outputs.len())
            .with_context(|| format!("task {}", call.task_id))?;
        if outputs.is_empty() {
            return Ok(Ok(ctx));
        }
        let values = call.outputs.iter().copied().zip(outputs).collect();
        Ok(Ok(view.put(ctx, values).await?))
    }

    /// Opens a scope with one branch per item of the list being fanned out. This worker owns the
    /// scope; the branch that fanned out waits in it until every item's branch has ended.
    async fn fan_out(
        self: &Arc<Self>,
        run: &Arc<Run>,
        op: OpId,
        ctx: NodeId,
        weight: Weight,
        scopes: Arc<[ScopeId]>,
    ) -> anyhow::Result<()> {
        let fan_out = kernel::operation(&run.spec.workflow, op)?
            .fan_out
            .as_ref()
            .context("op has no fan-out")?;
        let list = View { worker: self, run }.get(ctx, fan_out.input).await?;
        let Value::Array(items) = list else {
            bail!("fan-out over {list}, which is not a list");
        };
        let continuation = Continuation {
            ctx,
            weight,
            collect: run.spec.collect_ops[op],
            scopes: scopes.clone(),
        };
        if items.is_empty() {
            self.resume(run, continuation, Vec::new());
            return Ok(());
        }

        let scope = self.ids.next();
        let inner: Arc<[ScopeId]> = scopes.iter().copied().chain([scope]).collect();
        run.scopes
            .lock()
            .unwrap()
            .insert(scope, Scope::new(Then::Continue(continuation)));
        let shares = split(FULL, items.len());
        for (item, share) in items.into_iter().zip(shares) {
            let child = self.ids.next();
            run.nodes.insert(
                child,
                Arc::new(Node {
                    parent: Some(ctx),
                    values: vec![(fan_out.output, item)],
                }),
            );
            let branch = Branch {
                ctx: child,
                resume: Resume::Kernel(Start::After { op }),
                weight: share,
                scopes: inner.clone(),
            };
            let to = self.routes.any().unwrap_or(self.id);
            self.hand_off(run, to, branch);
        }
        Ok(())
    }

    /// Continues the branch that opened a scope, now that the scope is over.
    fn resume(self: &Arc<Self>, run: &Arc<Run>, continuation: Continuation, items: Vec<Value>) {
        let Continuation {
            ctx,
            weight,
            scopes,
            collect,
        } = continuation;
        let Some(op) = collect else {
            // Nothing collects this scope: the branch that opened it is done.
            let scope = *scopes
                .last()
                .expect("a continuation is inside the root scope");
            return self.end(run, scope, weight, None);
        };
        let output = run.spec.workflow.operations[op]
            .collect
            .as_ref()
            .expect("collect ops are found by scope analysis")
            .output;
        let node = self.ids.next();
        run.nodes.insert(
            node,
            Arc::new(Node {
                parent: Some(ctx),
                values: vec![(output, Value::Array(items))],
            }),
        );
        let branch = Branch {
            ctx: node,
            resume: Resume::Kernel(Start::After { op }),
            weight,
            scopes,
        };
        self.spawn_drive(run.clone(), branch);
    }

    /// Continues `branch` on worker `to`, with its context node if this worker holds it.
    fn hand_off(self: &Arc<Self>, run: &Arc<Run>, to: WorkerId, branch: Branch) {
        if to == self.id {
            return self.spawn_drive(run.clone(), branch);
        }
        let node = run.nodes.get(branch.ctx);
        let message = Message::Handoff {
            run: run.spec.id,
            branch,
            node,
        };
        self.transport.send(to, message);
    }

    /// Ends a branch of `scope`, returning its weight to the scope's owner.
    fn end(self: &Arc<Self>, run: &Arc<Run>, scope: ScopeId, weight: Weight, item: Option<Value>) {
        let to = owner(scope);
        if to == self.id {
            return self.on_end(run.spec.id, scope, weight, item);
        }
        let message = Message::End {
            run: run.spec.id,
            scope,
            weight,
            item,
        };
        self.transport.send(to, message);
    }

    fn on_end(
        self: &Arc<Self>,
        run_id: RunId,
        scope_id: ScopeId,
        weight: Weight,
        item: Option<Value>,
    ) {
        let Some(run) = self.run(run_id) else {
            return tracing::error!(%run_id, "branch end for an unknown run dropped");
        };
        let over = {
            let mut scopes = run.scopes.lock().unwrap();
            let Some(scope) = scopes.get_mut(&scope_id) else {
                return tracing::error!(%run_id, scope_id, "branch end for an unknown scope");
            };
            match scope.end(weight, item) {
                true => scopes.remove(&scope_id),
                false => None,
            }
        };
        let Some(scope) = over else { return };
        match scope.into_parts() {
            (_, Then::Finish { outcome, done }) => self.finish(&run, outcome, done),
            (items, Then::Continue(continuation)) => self.resume(&run, continuation, items),
        }
    }

    /// Sends a branch's report, with its weight, to the run's owner.
    fn report(self: &Arc<Self>, run: &Arc<Run>, scope: ScopeId, weight: Weight, report: Report) {
        if let Report::Failure(failure) = &report {
            tracing::warn!(run = %run.spec.id, "branch failed: {failure}");
        }
        let to = run.spec.owner();
        if to == self.id {
            return self.on_report(run.spec.id, scope, weight, report);
        }
        let message = Message::Report {
            run: run.spec.id,
            scope,
            weight,
            report,
        };
        self.transport.send(to, message);
    }

    fn on_report(self: &Arc<Self>, run_id: RunId, scope: ScopeId, weight: Weight, report: Report) {
        let Some(run) = self.run(run_id) else {
            return tracing::error!(%run_id, "report for an unknown run dropped");
        };
        {
            let mut scopes = run.scopes.lock().unwrap();
            let Some(Then::Finish { outcome, .. }) = scopes
                .get_mut(&run.spec.root_scope)
                .map(|root| &mut root.then)
            else {
                return tracing::error!(%run_id, "report for a run that is not running here");
            };
            match report {
                Report::Result(value) => outcome.results.push(value),
                Report::Failure(failure) => outcome.failures.push(failure),
            }
        }
        // Recorded: the branch's weight can now let its scope, and the run, end.
        self.end(&run, scope, weight, None);
    }

    fn finish(&self, run: &Run, outcome: RunOutcome, done: oneshot::Sender<RunOutcome>) {
        let run_id = run.spec.id;
        tracing::debug!(%run_id, "run finished");
        for worker in self.routes.workers() {
            if worker != self.id {
                self.transport.send(worker, Message::Forget { run: run_id });
            }
        }
        self.runs.lock().unwrap().remove(&run_id);
        let _ = done.send(outcome);
    }

    async fn call(&self, task: &str, input: Value) -> anyhow::Result<Result<Value, String>> {
        let _slot = self.task_slots.acquire().await?;
        // Tasks are CPU-bound; run them on this thread rather than hopping to the blocking pool.
        match Handle::current().runtime_flavor() {
            RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(|| self.tasks.call(task, input))
            }
            _ => self.tasks.call(task, input),
        }
    }

    /// A node of `run`, fetched from its owner if this worker does not hold it yet.
    async fn node(&self, run: &Run, id: NodeId) -> anyhow::Result<Arc<Node>> {
        if let Some(node) = run.nodes.get(id) {
            return Ok(node);
        }
        let holder = owner(id);
        ensure!(holder != self.id, "node {id} is missing");
        let request = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (waiter, reply) = oneshot::channel();
        self.fetches.lock().unwrap().insert(request, waiter);
        let message = Message::Fetch {
            run: run.spec.id,
            node: id,
            request,
            reply_to: self.id,
        };
        self.transport.send(holder, message);
        let node = reply
            .await
            .context("node fetch abandoned")?
            .with_context(|| format!("worker {holder} does not hold node {id}"))?;
        run.nodes.insert(id, node.clone());
        Ok(node)
    }
}

/// One run's value tree as the kernel sees it from this worker.
struct View<'a> {
    worker: &'a Inner,
    run: &'a Run,
}

impl ValueStore for View<'_> {
    async fn get(&self, ctx: NodeId, value: namu_core::ValueId) -> anyhow::Result<Value> {
        let mut cursor = Some(ctx);
        while let Some(id) = cursor {
            let node = self.worker.node(self.run, id).await?;
            if let Some(found) = node.get(value) {
                return Ok(found.clone());
            }
            cursor = node.parent;
        }
        bail!("value {value} is not visible from context {ctx}")
    }

    async fn put(
        &self,
        ctx: NodeId,
        values: Vec<(namu_core::ValueId, Value)>,
    ) -> anyhow::Result<NodeId> {
        let id = self.worker.ids.next();
        let node = Node {
            parent: Some(ctx),
            values,
        };
        self.run.nodes.insert(id, Arc::new(node));
        Ok(id)
    }
}
