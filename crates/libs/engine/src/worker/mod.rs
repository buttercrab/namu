//! A worker drives branches through the IR. It runs the tasks it serves itself, hands a branch to
//! another worker only when the next task is served elsewhere, owns the scopes it opens, and
//! serves the nodes it created.
//!
//! - A worker learns about a run when it first receives work for it, by asking the run's owner
//!   (named by the branch's root scope) for the run's description.
//! - Everything that moves responsibility for a branch is kept by its sender until the receiver has
//!   logged what it produced (`delivery`), and a worker that dies is taken over by its successor
//!   from the log (`recovery`). See design §7.
//! - When a run is over, its owner has every worker forget it, and reports the outcome.

mod batches;
mod delivery;
mod drive;
mod recovery;
mod scopes;
mod streams;
mod view;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use namu_core::ir::Workflow;
use tokio::sync::{Semaphore, mpsc, oneshot};

use self::delivery::Outgoing;
use self::scopes::Ending;
use crate::ids::{Minter, NodeId, RunId, ScopeId, WorkerId};
use crate::kernel::Start;
use crate::log::{Log, Record};
use crate::message::{Branch, BranchId, Message, Resume};
use crate::routes::Routes;
use crate::run::{RunInfo, RunOutcome, RunSpec, WorkerStats};
use crate::scope::{Finish, Scope, Then};
use crate::tasks::{TaskRef, Tasks};
use crate::transport::Transport;
use crate::tree::{Node, Nodes};

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
    log: Arc<dyn Log>,
    runs: Mutex<HashMap<RunId, Arc<Run>>>,
    /// Runs whose description is being fetched, with the messages waiting for it. Lock before
    /// `runs`.
    pending: Mutex<HashMap<RunId, PendingRun>>,
    /// Dead workers taken over, and messages waiting for a takeover or a membership change.
    adoption: Mutex<recovery::Adoption>,
    /// Runs this worker owns that are over, waiting for the other workers to forget them.
    ending: Mutex<HashMap<RunId, Ending>>,
    /// Nodes requested from their holders, with everyone waiting for each.
    fetches: Mutex<HashMap<(RunId, NodeId), Vec<NodeWaiter>>>,
    /// Messages kept until their receiver acknowledges them.
    outbox: Mutex<HashMap<u64, Outgoing>>,
    next_seq: AtomicU64,
    /// Bounds concurrent task calls; driving the IR between them is cheap and unbounded.
    task_slots: Semaphore,
    /// Batch tasks' queues and sessions, per task version.
    batchers: Mutex<HashMap<TaskRef, Arc<batches::Batcher>>>,
}

/// A run as one worker sees it.
struct Run {
    spec: Arc<RunSpec>,
    nodes: Arc<Nodes>,
    /// The scopes this worker owns that are open: opened here, or taken over from a dead worker.
    scopes: Mutex<HashMap<ScopeId, Scope>>,
    calls: AtomicU64,
    fetches: AtomicU64,
}

struct PendingRun {
    /// The run's owner, as named by the first message.
    owner: WorkerId,
    /// Who was asked: the owner, or its successor if it died.
    asked: WorkerId,
    messages: Vec<Message>,
}

impl Worker {
    /// Starts a worker that receives its messages from `inbox` and runs up to `task_slots` task
    /// calls at once.
    pub fn spawn(
        id: WorkerId,
        tasks: Arc<dyn Tasks>,
        routes: Arc<Routes>,
        transport: Arc<dyn Transport>,
        log: Arc<dyn Log>,
        mut inbox: mpsc::UnboundedReceiver<Message>,
        task_slots: usize,
    ) -> Self {
        let inner = Arc::new(Inner {
            id,
            ids: Minter::new(id),
            tasks,
            routes,
            transport,
            log,
            runs: Mutex::default(),
            pending: Mutex::default(),
            adoption: Mutex::default(),
            ending: Mutex::default(),
            fetches: Mutex::default(),
            outbox: Mutex::default(),
            next_seq: AtomicU64::new(0),
            task_slots: Semaphore::new(task_slots),
            batchers: Mutex::default(),
        });
        let worker = inner.clone();
        tokio::spawn(async move {
            while let Some(message) = inbox.recv().await {
                worker.receive(message);
            }
        });
        tokio::spawn(inner.clone().watch_members());
        Self { inner }
    }

    pub fn id(&self) -> WorkerId {
        self.inner.id
    }

    /// One task call per CPU.
    pub fn default_task_slots() -> usize {
        std::thread::available_parallelism().map_or(4, |n| n.get())
    }

    pub fn serves(&self, task: &TaskRef) -> bool {
        self.inner.tasks.task(task).is_some()
    }
}

impl Inner {
    fn run_state(&self, id: RunId) -> Option<Arc<Run>> {
        self.runs.lock().unwrap().get(&id).cloned()
    }

    /// Sends `message` to `to`, or handles it here if `to` is this worker.
    fn deliver(self: &Arc<Self>, to: WorkerId, message: Message) {
        match to == self.id {
            true => self.receive(message),
            false => self.transport.send(to, message),
        }
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
                    worker.start(run, workflow, task_versions, reply_to).await;
                });
            }
            Message::Finished { run, .. } => {
                tracing::warn!(%run, "run outcome sent to a worker; only the control plane takes them");
            }
            message @ Message::Handoff { .. } => self.on_handoff(message),
            message @ Message::End { .. } => self.on_end(message),
            message @ Message::Report { .. } => self.on_report(message),
            Message::Ack { seqs } => self.on_ack(seqs),
            Message::FetchNode { run, id, reply_to } => {
                let node = self.run_state(run).and_then(|state| state.nodes.get(id));
                self.deliver(reply_to, Message::Node { run, id, node });
            }
            Message::Node { run, id, node } => self.on_node(run, id, node),
            message @ Message::FetchRun { .. } => self.on_fetch_run(message),
            Message::Run { info } => {
                let worker = self.clone();
                tokio::spawn(async move { worker.install(info).await });
            }
            Message::RunOver { run } => self.on_run_over(run),
            Message::Forget { run, reply_to } => self.on_forget(run, reply_to),
            Message::Forgotten { run, worker, stats } => self.on_forgotten(run, worker, stats),
        }
    }

    /// Starts a run owned by this worker, unless it is already running (a retried `Start`).
    async fn start(
        self: &Arc<Self>,
        id: RunId,
        workflow: Workflow,
        task_versions: BTreeMap<String, String>,
        reply_to: WorkerId,
    ) {
        let known = self.run_state(id).is_some();
        if known || matches!(self.log.run(id).await, Ok(Some(_))) {
            return tracing::info!(%id, "run already started; its owner's successor carries it on");
        }
        let info = Arc::new(RunInfo {
            id,
            workflow,
            task_versions,
            root: self.ids.next(),
            root_scope: self.ids.next(),
            reply_to: Some(reply_to),
        });
        let spec = match RunSpec::from_info(info.clone()) {
            Ok(spec) => Arc::new(spec),
            Err(err) => {
                let outcome = RunOutcome {
                    failures: vec![format!("run did not start: {err:#}")],
                    ..RunOutcome::default()
                };
                return self.deliver(reply_to, Message::Finished { run: id, outcome });
            }
        };
        if let Err(err) = self.tasks.prepare(&spec).await {
            tracing::error!(%id, "preparing tasks failed: {err:#}");
        }
        let run = self.install_spec(spec);

        let root = run.nodes.hold(Arc::new(Node::root(info.root)));
        let finish = Finish {
            outcome: RunOutcome::default(),
            reported: HashSet::new(),
        };
        // The root scope has one branch: the run's root branch.
        let root_scope = Scope::new(Then::Finish(Box::new(finish)), Some(1));
        run.open_scope(info.root_scope, root_scope);
        let branch = Branch {
            id: BranchId {
                scope: info.root_scope,
                index: 0,
            },
            hop: 0,
            ctx: info.root,
            resume: Resume::Kernel(Start::At { op: 0, pred: None }),
            scopes: Arc::from([info.root_scope]),
        };
        // The run is durable before any branch can leave this worker. A run whose owner dies
        // before that is unknown everywhere else, so starting it again is clean.
        let durable = self.log.append(vec![
            Record::Run(info.clone()),
            Record::Nodes {
                run: id,
                nodes: vec![root.clone()],
            },
            Record::Scope {
                run: id,
                scope: info.root_scope,
                continuation: None,
                source: None,
                count: Some(1),
            },
            Record::Branch {
                run: id,
                holder: self.id,
                branch: branch.clone(),
            },
        ]);
        if durable.await.is_err() {
            return tracing::error!(%id, "the run could not be logged; it is started again elsewhere");
        }
        self.spawn_drive(run, branch, root);
    }

    /// Makes a run described by its owner known here, then replays the messages that waited.
    async fn install(self: &Arc<Self>, info: Arc<RunInfo>) -> Option<Arc<Run>> {
        if let Some(run) = self.run_state(info.id) {
            return Some(run);
        }
        let id = info.id;
        let spec = match RunSpec::from_info(info) {
            Ok(spec) => Arc::new(spec),
            Err(err) => {
                tracing::error!(%id, "unusable run description: {err:#}");
                return None;
            }
        };
        // Branches still run if preparing fails; the calls it was for fail instead.
        if let Err(err) = self.tasks.prepare(&spec).await {
            tracing::error!(%id, "preparing tasks failed: {err:#}");
        }
        Some(self.install_spec(spec))
    }

    fn install_spec(self: &Arc<Self>, spec: Arc<RunSpec>) -> Arc<Run> {
        let id = spec.id();
        let (run, waiting) = {
            let mut pending = self.pending.lock().unwrap();
            let mut runs = self.runs.lock().unwrap();
            let run = runs
                .entry(id)
                .or_insert_with(|| {
                    Arc::new(Run {
                        spec,
                        nodes: Arc::default(),
                        scopes: Mutex::default(),
                        calls: AtomicU64::new(0),
                        fetches: AtomicU64::new(0),
                    })
                })
                .clone();
            let waiting = pending.remove(&id).map(|p| p.messages).unwrap_or_default();
            (run, waiting)
        };
        for message in waiting {
            self.receive(message);
        }
        run
    }

    /// The run if it is known here. Otherwise `message` waits for the run's description, which is
    /// requested from `owner` on first contact.
    fn known_or_wait(
        self: &Arc<Self>,
        run: RunId,
        owner: WorkerId,
        message: Message,
    ) -> Option<(Arc<Run>, Message)> {
        let mut pending = self.pending.lock().unwrap();
        if let Some(state) = self.run_state(run) {
            return Some((state, message));
        }
        let asked = self.routes.resolve(owner);
        let entry = pending.entry(run).or_insert_with(|| PendingRun {
            owner,
            asked,
            messages: Vec::new(),
        });
        let first = entry.messages.is_empty();
        entry.messages.push(message);
        drop(pending);
        if first {
            let reply_to = self.id;
            self.deliver(asked, Message::FetchRun { run, reply_to });
        }
        None
    }

    fn on_fetch_run(self: &Arc<Self>, message: Message) {
        let Message::FetchRun { run, reply_to } = message else {
            unreachable!()
        };
        if let Some(state) = self.run_state(run) {
            let info = state.spec.info.clone();
            return self.deliver(reply_to, Message::Run { info });
        }
        // Not known here: the run is over, or this worker is about to take it over.
        self.orphan(message);
    }

    /// The run is over: messages that waited for it are acknowledged and dropped.
    fn on_run_over(self: &Arc<Self>, run: RunId) {
        let waiting = self.pending.lock().unwrap().remove(&run);
        for message in waiting.map(|p| p.messages).unwrap_or_default() {
            self.drop_late(message);
        }
    }

    /// Acknowledges a message that arrived after its run or scope was over, and drops it.
    fn drop_late(self: &Arc<Self>, message: Message) {
        if let Some(delivery) = message.delivery() {
            self.ack(delivery);
        }
    }
}

impl Run {
    /// Owns `scope` here.
    fn open_scope(&self, id: ScopeId, scope: Scope) {
        self.scopes.lock().unwrap().insert(id, scope);
    }

    fn stats(&self) -> WorkerStats {
        WorkerStats {
            calls: self.calls.load(Ordering::Relaxed),
            fetches: self.fetches.load(Ordering::Relaxed),
            peak_nodes: self.nodes.peak() as u64,
        }
    }

    fn id(&self) -> RunId {
        self.spec.id()
    }
}
