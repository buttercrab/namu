//! Workers in one process: the reference configuration.
//!
//! It is the same engine that runs across machines, with channels (or TCP on localhost) between
//! workers, an in-memory log, and itself as the control plane (peer 0). Every workflow must
//! produce the same results here as on any number of machines, which is what makes it the
//! standard the distributed runtime is checked against. Workers can also be killed, to check that
//! a run survives a crash.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Context;
use async_trait::async_trait;
use namu_core::ir::{Operation, Workflow};
use tokio::sync::{mpsc, oneshot};

use crate::ids::{NodeId, RunId, WorkerId};
use crate::log::{Durable, Log, MemoryLog, Record, RunEstate};
use crate::message::Message;
use crate::routes::Routes;
use crate::run::{RunInfo, RunOutcome};
use crate::tasks::{TaskRef, Tasks};
use crate::transport::{LocalTransport, TcpTransport, Transport};
use crate::tree::Node;
use crate::worker::Worker;

/// The control plane's id in the mesh.
const CONTROL: WorkerId = 0;

pub struct LocalCluster {
    workers: Vec<Worker>,
    routes: Arc<Routes>,
    control: Arc<dyn Transport>,
    waiting: Arc<Mutex<HashMap<RunId, oneshot::Sender<RunOutcome>>>>,
    /// Runs that have not finished: who owns them and how to start them again.
    running: Arc<Mutex<HashMap<RunId, (WorkerId, Workflow)>>>,
    /// Workers draining: still running, but given no new work.
    leaving: Mutex<HashSet<WorkerId>>,
    dead: Arc<Dead>,
    successors: Mutex<HashMap<WorkerId, WorkerId>>,
    tasks: Mutex<BTreeSet<TaskRef>>,
    next_owner: AtomicUsize,
    log: Arc<MemoryLog>,
}

/// Workers that were killed: nothing they send or log arrives anywhere.
#[derive(Debug, Default)]
struct Dead(RwLock<HashSet<WorkerId>>);

impl Dead {
    fn contains(&self, worker: WorkerId) -> bool {
        self.0.read().unwrap().contains(&worker)
    }
}

type Endpoint = (Arc<dyn Transport>, mpsc::UnboundedReceiver<Message>);

impl LocalCluster {
    /// Starts one worker per task set, connected by channels. Must be called within a Tokio
    /// runtime.
    pub fn new(task_sets: Vec<Arc<dyn Tasks>>) -> Self {
        let transport = Arc::new(LocalTransport::default());
        let mut connect = |id: WorkerId| -> Endpoint {
            let inbox = transport.connect(id);
            (transport.clone(), inbox)
        };
        let control = connect(CONTROL);
        let endpoints = (1..=task_sets.len() as WorkerId)
            .map(&mut connect)
            .collect();
        Self::assemble(task_sets, control, endpoints)
    }

    /// Like [`LocalCluster::new`], but the workers talk over TCP on localhost, so every message
    /// is encoded, sent and decoded exactly as between machines.
    pub async fn tcp(task_sets: Vec<Arc<dyn Tasks>>) -> std::io::Result<Self> {
        let mut transports = Vec::new();
        for _ in 0..=task_sets.len() {
            transports.push(TcpTransport::bind(([127, 0, 0, 1], 0).into()).await?);
        }
        for (transport, _, _) in &transports {
            for (id, (_, addr, _)) in (0..).zip(&transports) {
                transport.set_peer(id, *addr);
            }
        }
        let mut endpoints: Vec<Endpoint> = transports
            .into_iter()
            .map(|(transport, _, inbox)| (transport as Arc<dyn Transport>, inbox))
            .collect();
        let control = endpoints.remove(0);
        Ok(Self::assemble(task_sets, control, endpoints))
    }

    /// Starts `n` workers that all serve `tasks`.
    pub fn uniform(n: usize, tasks: Arc<dyn Tasks>) -> Self {
        Self::new(vec![tasks; n])
    }

    fn assemble(
        task_sets: Vec<Arc<dyn Tasks>>,
        control: Endpoint,
        endpoints: Vec<Endpoint>,
    ) -> Self {
        assert!(!task_sets.is_empty(), "a cluster needs a worker");
        let dead = Arc::new(Dead::default());
        let log = Arc::new(MemoryLog::default());
        let routes = Arc::new(Routes::default());
        let workers: Vec<Worker> = (1..)
            .zip(task_sets.into_iter().zip(endpoints))
            .map(|(id, (tasks, (transport, inbox)))| {
                let transport = Arc::new(Mortal {
                    worker: id,
                    inner: transport,
                    dead: dead.clone(),
                });
                let log = Arc::new(MortalLog {
                    worker: id,
                    inner: log.clone(),
                    dead: dead.clone(),
                });
                let slots = Worker::default_task_slots();
                Worker::spawn(id, tasks, routes.clone(), transport, log, inbox, slots)
            })
            .collect();
        routes.set_workers(workers.iter().map(Worker::id).collect());

        let waiting: Arc<Mutex<HashMap<RunId, oneshot::Sender<RunOutcome>>>> = Arc::default();
        let running: Arc<Mutex<HashMap<RunId, (WorkerId, Workflow)>>> = Arc::default();
        let (control, mut inbox) = control;
        let (finished, over) = (waiting.clone(), running.clone());
        tokio::spawn(async move {
            while let Some(message) = inbox.recv().await {
                if let Message::Finished { run, outcome } = message {
                    over.lock().unwrap().remove(&run);
                    if let Some(waiter) = finished.lock().unwrap().remove(&run) {
                        let _ = waiter.send(outcome);
                    }
                }
            }
        });
        Self {
            workers,
            routes,
            control,
            waiting,
            running,
            dead,
            successors: Mutex::default(),
            leaving: Mutex::default(),
            tasks: Mutex::default(),
            next_owner: AtomicUsize::new(0),
            log,
        }
    }

    /// The log the workers share.
    pub fn log(&self) -> &MemoryLog {
        &self.log
    }

    /// The workers that take work: neither dead nor leaving.
    fn live(&self) -> Vec<&Worker> {
        let leaving = self.leaving.lock().unwrap();
        self.workers
            .iter()
            .filter(|worker| !self.dead.contains(worker.id()) && !leaving.contains(&worker.id()))
            .collect()
    }

    /// Points routes at the live workers.
    fn reroute(&self) -> anyhow::Result<()> {
        let live = self.live();
        self.routes
            .set_workers(live.iter().map(|w| w.id()).collect());
        for task in self.tasks.lock().unwrap().iter() {
            let servers: Vec<WorkerId> = live
                .iter()
                .filter(|w| w.serves(task))
                .map(|w| w.id())
                .collect();
            // Membership is fixed here, so a branch waiting for a server would wait forever.
            anyhow::ensure!(
                !servers.is_empty(),
                "no live worker serves task {}",
                task.id
            );
            self.routes.set_servers(task.clone(), servers);
        }
        Ok(())
    }

    /// Runs `workflow` to completion.
    pub async fn run(&self, workflow: Workflow) -> anyhow::Result<RunOutcome> {
        let tasks = workflow
            .operations
            .iter()
            .filter_map(Operation::call)
            .map(|call| TaskRef {
                id: call.task_id.clone(),
                version: String::new(),
            });
        self.tasks.lock().unwrap().extend(tasks);
        self.reroute()?;

        let live = self.live();
        let i = self.next_owner.fetch_add(1, Ordering::Relaxed);
        let owner = live[i % live.len()].id();
        let run = RunId::new_v4();
        let (done, outcome) = oneshot::channel();
        self.waiting.lock().unwrap().insert(run, done);
        self.running
            .lock()
            .unwrap()
            .insert(run, (owner, workflow.clone()));
        self.start(owner, run, workflow);
        outcome.await.context("run abandoned")
    }

    fn start(&self, owner: WorkerId, run: RunId, workflow: Workflow) {
        let start = Message::Start {
            run,
            workflow,
            task_versions: BTreeMap::new(),
            reply_to: CONTROL,
        };
        self.control.send(owner, start);
    }

    /// Kills a worker, as a crash would: from now on nothing it sends or logs arrives. Its
    /// successor is the lowest-numbered live worker.
    /// Drains `worker` and lets it leave (design §10.4): work is routed around it, it hands on
    /// what it holds, and then its successor takes over what it owns, as for a crash but with
    /// nothing to run again.
    pub async fn drain(&self, worker: WorkerId) -> anyhow::Result<()> {
        let draining = self
            .workers
            .iter()
            .find(|w| w.id() == worker)
            .context("no such worker")?;
        self.leaving.lock().unwrap().insert(worker);
        self.reroute()?;
        draining.drain().await?;
        self.kill(worker)
    }

    pub fn kill(&self, worker: WorkerId) -> anyhow::Result<()> {
        self.dead.0.write().unwrap().insert(worker);
        let successor = self.live().first().context("no worker left alive")?.id();
        let mut successors = self.successors.lock().unwrap();
        // Workers it had taken over from follow it to its successor.
        successors.insert(worker, successor);
        self.routes.set_successors(successors.clone());
        drop(successors);
        self.reroute()?;
        // As the orchestrator does: a run whose owner died before logging it starts again on the
        // successor, which ignores the start if it can take the run over from the log instead.
        let orphaned: Vec<(RunId, Workflow)> = self
            .running
            .lock()
            .unwrap()
            .iter_mut()
            .filter(|(_, (owner, _))| *owner == worker)
            .map(|(&run, (owner, workflow))| {
                *owner = successor;
                (run, workflow.clone())
            })
            .collect();
        for (run, workflow) in orphaned {
            self.start(successor, run, workflow);
        }
        Ok(())
    }
}

/// A worker's transport, which goes silent when the worker is killed.
struct Mortal {
    worker: WorkerId,
    inner: Arc<dyn Transport>,
    dead: Arc<Dead>,
}

impl Transport for Mortal {
    fn send(&self, to: WorkerId, message: Message) {
        if !self.dead.contains(self.worker) && !self.dead.contains(to) {
            self.inner.send(to, message);
        }
    }
}

/// A worker's view of the shared log; a killed worker's appends never land.
struct MortalLog {
    worker: WorkerId,
    inner: Arc<MemoryLog>,
    dead: Arc<Dead>,
}

#[async_trait]
impl Log for MortalLog {
    fn append(&self, records: Vec<Record>) -> Durable {
        if self.dead.contains(self.worker) {
            return oneshot::channel().1;
        }
        self.inner.append(records)
    }

    async fn node(&self, run: RunId, id: NodeId) -> anyhow::Result<Option<Arc<Node>>> {
        self.inner.node(run, id).await
    }

    async fn run(&self, run: RunId) -> anyhow::Result<Option<Arc<RunInfo>>> {
        self.inner.run(run).await
    }

    async fn estate(&self, worker: WorkerId) -> anyhow::Result<Vec<RunEstate>> {
        self.inner.estate(worker).await
    }
}
