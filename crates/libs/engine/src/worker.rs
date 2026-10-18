//! A worker drives branches through the IR. It runs the tasks it serves itself, hands a branch to
//! another worker only when the next task is served elsewhere, owns the scopes it opens, and
//! serves the nodes it created.
//!
//! A worker learns about a run when it first receives work for it: it asks the run's owner (named
//! by the branch's root scope) for the run's description, holds the run's messages until the
//! answer arrives, and prepares its tasks. When a run is over, its owner tells every worker to
//! forget it, and the run ends once they have.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use namu_core::OpId;
use namu_core::ir::{Call, Workflow};
use serde_json::Value;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::ids::{Minter, NodeId, RunId, ScopeId, WorkerId, owner};
use crate::kernel::{self, Plan, Start, ValueStore};
use crate::message::{Branch, Message, Report, Resume};
use crate::routes::Routes;
use crate::run::{RunInfo, RunOutcome, RunSpec, WorkerStats};
use crate::scope::{Continuation, FULL, Scope, Then, Weight, split};
use crate::tasks::{TaskRef, Tasks, pack_input, unpack_output};
use crate::transport::Transport;
use crate::tree::{Node, Nodes};

/// How long the owner of a finished run waits for every worker to confirm it forgot the run.
const FORGET_PATIENCE: Duration = Duration::from_secs(2);

type NodeWaiter = oneshot::Sender<Option<Arc<Node>>>;

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
    /// Messages for runs whose description is being fetched from their owner. Lock before `runs`.
    pending: Mutex<HashMap<RunId, Vec<Message>>>,
    /// Runs this worker owns that are over, waiting for the other workers to forget them.
    ending: Mutex<HashMap<RunId, Ending>>,
    /// Nodes requested from their owners, with everyone waiting for each. One request per node,
    /// however many branches need it at once.
    fetches: Mutex<HashMap<(RunId, NodeId), Vec<NodeWaiter>>>,
    /// Bounds concurrent task calls; driving the IR between them is cheap and unbounded.
    task_slots: Semaphore,
}

/// A run as one worker sees it.
struct Run {
    spec: Arc<RunSpec>,
    nodes: Nodes,
    /// The scopes this worker opened and owns.
    scopes: Mutex<HashMap<ScopeId, Scope>>,
    calls: AtomicU64,
    fetches: AtomicU64,
}

struct Ending {
    outcome: RunOutcome,
    waiting: HashSet<WorkerId>,
    done: oneshot::Sender<RunOutcome>,
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
            pending: Mutex::new(HashMap::new()),
            ending: Mutex::new(HashMap::new()),
            fetches: Mutex::new(HashMap::new()),
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

    pub fn serves(&self, task: &TaskRef) -> bool {
        self.inner.tasks.serves(task)
    }

    /// Runs `workflow`, owned by this worker, to completion.
    pub async fn run(
        &self,
        workflow: Workflow,
        task_versions: BTreeMap<String, String>,
    ) -> anyhow::Result<RunOutcome> {
        self.inner
            .run(RunId::new_v4(), workflow, task_versions)
            .await
    }
}

impl Inner {
    fn run_state(&self, id: RunId) -> Option<Arc<Run>> {
        self.runs.lock().unwrap().get(&id).cloned()
    }

    async fn run(
        self: &Arc<Self>,
        id: RunId,
        workflow: Workflow,
        task_versions: BTreeMap<String, String>,
    ) -> anyhow::Result<RunOutcome> {
        let spec = RunSpec::from_info(RunInfo {
            id,
            workflow,
            task_versions,
            root: self.ids.next(),
            root_scope: self.ids.next(),
        })?;
        let spec = Arc::new(spec);
        self.tasks.prepare(&spec).await?;
        let run = self.install(spec.clone());

        let (done, outcome) = oneshot::channel();
        let root = Node {
            parent: None,
            values: Vec::new(),
        };
        run.nodes.insert(spec.info.root, Arc::new(root));
        let finish = Then::Finish {
            outcome: RunOutcome::default(),
            done,
        };
        run.scopes
            .lock()
            .unwrap()
            .insert(spec.info.root_scope, Scope::new(finish));
        let branch = Branch {
            ctx: spec.info.root,
            resume: Resume::Kernel(Start::At { op: 0, pred: None }),
            weight: FULL,
            scopes: Arc::from([spec.info.root_scope]),
        };
        self.spawn_drive(run, branch);
        outcome.await.context("run abandoned")
    }

    /// Makes `spec` known here and replays the messages that waited for it.
    fn install(self: &Arc<Self>, spec: Arc<RunSpec>) -> Arc<Run> {
        let run = Arc::new(Run {
            spec,
            nodes: Nodes::default(),
            scopes: Mutex::new(HashMap::new()),
            calls: AtomicU64::new(0),
            fetches: AtomicU64::new(0),
        });
        let waiting = {
            let mut pending = self.pending.lock().unwrap();
            self.runs.lock().unwrap().insert(run.spec.id(), run.clone());
            pending.remove(&run.spec.id()).unwrap_or_default()
        };
        for message in waiting {
            self.receive(message);
        }
        run
    }

    fn receive(self: &Arc<Self>, message: Message) {
        match message {
            Message::Start {
                run,
                workflow,
                task_versions,
                reply_to,
            } => {
                let worker = self.clone();
                tokio::spawn(async move {
                    let outcome = match worker.run(run, workflow, task_versions).await {
                        Ok(outcome) => outcome,
                        Err(err) => RunOutcome {
                            failures: vec![format!("run did not start: {err:#}")],
                            ..RunOutcome::default()
                        },
                    };
                    let finished = Message::Finished { run, outcome };
                    worker.transport.send(reply_to, finished);
                });
            }
            Message::Finished { run, .. } => {
                tracing::warn!(%run, "run outcome sent to a worker; only the control plane takes them");
            }
            Message::Handoff { run, branch, nodes } => {
                let state = {
                    let mut pending = self.pending.lock().unwrap();
                    match self.run_state(run) {
                        Some(state) => state,
                        None => {
                            let owner = owner(branch.scopes[0]);
                            let handoff = Message::Handoff { run, branch, nodes };
                            return self.park(&mut pending, run, owner, handoff);
                        }
                    }
                };
                for (id, node) in nodes {
                    state.nodes.insert(id, node);
                }
                self.spawn_drive(state, branch);
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
            Message::FetchNode { run, id, reply_to } => {
                let node = self.run_state(run).and_then(|state| state.nodes.get(id));
                self.transport
                    .send(reply_to, Message::Node { run, id, node });
            }
            Message::Node { run, id, node } => {
                let waiters = self.fetches.lock().unwrap().remove(&(run, id));
                for waiter in waiters.into_iter().flatten() {
                    let _ = waiter.send(node.clone());
                }
            }
            Message::FetchRun { run, reply_to } => match self.run_state(run) {
                Some(run) => {
                    let info = Box::new(run.spec.info.clone());
                    self.transport.send(reply_to, Message::Run { info });
                }
                None => tracing::error!(%run, "asked to describe a run not owned here"),
            },
            Message::Run { info } => {
                let worker = self.clone();
                tokio::spawn(async move {
                    let id = info.id;
                    let spec = match RunSpec::from_info(*info) {
                        Ok(spec) => Arc::new(spec),
                        Err(err) => {
                            return tracing::error!(%id, "unusable run description: {err:#}");
                        }
                    };
                    // Branches still run if preparing fails; the calls it was for fail instead.
                    if let Err(err) = worker.tasks.prepare(&spec).await {
                        tracing::error!(%id, "preparing tasks failed: {err:#}");
                    }
                    worker.install(spec);
                });
            }
            Message::Forget { run, reply_to } => {
                self.pending.lock().unwrap().remove(&run);
                let stats = self
                    .runs
                    .lock()
                    .unwrap()
                    .remove(&run)
                    .map(|run| run.stats())
                    .unwrap_or_default();
                let worker = self.id;
                self.transport
                    .send(reply_to, Message::Forgotten { run, worker, stats });
            }
            Message::Forgotten { run, worker, stats } => self.on_forgotten(run, worker, stats),
        }
    }

    /// Holds `message` until `run`, unknown here, is described by its owner. The first message
    /// for a run asks for the description.
    fn park(
        &self,
        pending: &mut HashMap<RunId, Vec<Message>>,
        run: RunId,
        owner: WorkerId,
        message: Message,
    ) {
        let waiting = pending.entry(run).or_default();
        if waiting.is_empty() {
            let reply_to = self.id;
            self.transport
                .send(owner, Message::FetchRun { run, reply_to });
        }
        waiting.push(message);
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
        let arrived = ctx;
        let workflow = run.spec.workflow();
        let view = View { worker: self, run };
        loop {
            let plan = match resume {
                Resume::Kernel(start) => kernel::drive(workflow, &view, ctx, start).await?,
                Resume::Call { op } => Plan::Call { op, ctx },
            };
            match plan {
                Plan::Call { op, ctx: at } => {
                    let task = run.spec.task(op).context("op has no call")?;
                    if !self.tasks.serves(task) {
                        let to = self.routes.wait_for_server(task).await;
                        let resume = Resume::Call { op };
                        let branch = Branch {
                            ctx: at,
                            resume,
                            weight,
                            scopes,
                        };
                        self.hand_off(run, to, branch, arrived);
                        return Ok(());
                    }
                    let call = kernel::operation(workflow, op)?
                        .call
                        .as_ref()
                        .context("op has no call")?;
                    match self.run_call(&view, task, call, at).await? {
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
        task: &TaskRef,
        call: &Call,
        ctx: NodeId,
    ) -> anyhow::Result<Result<NodeId, String>> {
        let mut args = Vec::with_capacity(call.inputs.len());
        for &input in &call.inputs {
            args.push(view.get(ctx, input).await?);
        }
        let output = self.call(task, pack_input(args)).await?;
        view.run.calls.fetch_add(1, Ordering::Relaxed);
        let output = match output {
            Ok(output) => output,
            Err(message) => return Ok(Err(format!("{}: {message}", task.id))),
        };
        let outputs = unpack_output(output, call.outputs.len())
            .with_context(|| format!("task {}", task.id))?;
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
        let fan_out = kernel::operation(run.spec.workflow(), op)?
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
            self.hand_off(run, to, branch, child);
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
        let output = run.spec.workflow().operations[op]
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

    /// Continues `branch` on worker `to`, with the nodes of its context back to `since` (the
    /// context it arrived here with) that this worker holds.
    fn hand_off(self: &Arc<Self>, run: &Arc<Run>, to: WorkerId, branch: Branch, since: NodeId) {
        if to == self.id {
            return self.spawn_drive(run.clone(), branch);
        }
        let mut nodes = Vec::new();
        let mut cursor = Some(branch.ctx);
        while let Some(id) = cursor {
            let Some(node) = run.nodes.get(id) else { break };
            cursor = node.parent.filter(|_| id != since);
            nodes.push((id, node));
        }
        let message = Message::Handoff {
            run: run.spec.id(),
            branch,
            nodes,
        };
        self.transport.send(to, message);
    }

    /// Ends a branch of `scope`, returning its weight to the scope's owner.
    fn end(self: &Arc<Self>, run: &Arc<Run>, scope: ScopeId, weight: Weight, item: Option<Value>) {
        let to = owner(scope);
        if to == self.id {
            return self.on_end(run.spec.id(), scope, weight, item);
        }
        let message = Message::End {
            run: run.spec.id(),
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
        let Some(run) = self.run_state(run_id) else {
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
            tracing::warn!(run = %run.spec.id(), "branch failed: {failure}");
        }
        let to = run.spec.owner();
        if to == self.id {
            return self.on_report(run.spec.id(), scope, weight, report);
        }
        let message = Message::Report {
            run: run.spec.id(),
            scope,
            weight,
            report,
        };
        self.transport.send(to, message);
    }

    fn on_report(self: &Arc<Self>, run_id: RunId, scope: ScopeId, weight: Weight, report: Report) {
        let Some(run) = self.run_state(run_id) else {
            return tracing::error!(%run_id, "report for an unknown run dropped");
        };
        {
            let mut scopes = run.scopes.lock().unwrap();
            let root = scopes.get_mut(&run.spec.info.root_scope);
            let Some(Then::Finish { outcome, .. }) = root.map(|root| &mut root.then) else {
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

    /// Every branch of the run has ended: have every worker forget it, then resolve the run.
    fn finish(
        self: &Arc<Self>,
        run: &Run,
        mut outcome: RunOutcome,
        done: oneshot::Sender<RunOutcome>,
    ) {
        let run_id = run.spec.id();
        self.runs.lock().unwrap().remove(&run_id);
        record(&mut outcome, self.id, run.stats());
        let waiting: HashSet<WorkerId> = self
            .routes
            .workers()
            .into_iter()
            .filter(|&worker| worker != self.id)
            .collect();
        // Recorded before asking, so no reply can arrive before it is expected.
        let ending = Ending {
            outcome,
            waiting: waiting.clone(),
            done,
        };
        self.ending.lock().unwrap().insert(run_id, ending);
        for worker in waiting {
            let reply_to = self.id;
            self.transport.send(
                worker,
                Message::Forget {
                    run: run_id,
                    reply_to,
                },
            );
        }
        self.on_forgotten_by_all(run_id);

        let worker = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(FORGET_PATIENCE).await;
            let Some(ending) = worker.ending.lock().unwrap().remove(&run_id) else {
                return;
            };
            tracing::warn!(%run_id, missing = ?ending.waiting, "workers did not confirm the end of the run");
            let _ = ending.done.send(ending.outcome);
        });
    }

    fn on_forgotten(&self, run_id: RunId, worker: WorkerId, stats: WorkerStats) {
        if let Some(ending) = self.ending.lock().unwrap().get_mut(&run_id) {
            record(&mut ending.outcome, worker, stats);
            ending.waiting.remove(&worker);
        }
        self.on_forgotten_by_all(run_id);
    }

    fn on_forgotten_by_all(&self, run_id: RunId) {
        let mut ending = self.ending.lock().unwrap();
        if ending.get(&run_id).is_some_and(|e| e.waiting.is_empty()) {
            let ending = ending.remove(&run_id).expect("checked above");
            tracing::debug!(%run_id, "run finished");
            let _ = ending.done.send(ending.outcome);
        }
    }

    async fn call(&self, task: &TaskRef, input: Value) -> anyhow::Result<Result<Value, String>> {
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
        let (waiter, reply) = oneshot::channel();
        let first = {
            let mut fetches = self.fetches.lock().unwrap();
            let waiters = fetches.entry((run.spec.id(), id)).or_default();
            waiters.push(waiter);
            waiters.len() == 1
        };
        if first {
            let message = Message::FetchNode {
                run: run.spec.id(),
                id,
                reply_to: self.id,
            };
            self.transport.send(holder, message);
            run.fetches.fetch_add(1, Ordering::Relaxed);
        }
        let node = reply
            .await
            .context("node fetch abandoned")?
            .with_context(|| format!("worker {holder} does not hold node {id}"))?;
        run.nodes.insert(id, node.clone());
        Ok(node)
    }
}

impl Run {
    fn stats(&self) -> WorkerStats {
        WorkerStats {
            calls: self.calls.load(Ordering::Relaxed),
            fetches: self.fetches.load(Ordering::Relaxed),
        }
    }
}

/// Adds a worker's stats to an outcome, if it did anything for the run.
fn record(outcome: &mut RunOutcome, worker: WorkerId, stats: WorkerStats) {
    if stats != WorkerStats::default() {
        outcome.workers.insert(worker, stats);
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
