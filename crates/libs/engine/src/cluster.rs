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
use namu_core::ValueId;
use namu_core::inputs::Inputs;
use namu_core::ir::{Operation, Workflow};
use namu_core::signature::Signature;
use namu_core::value::Value;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Duration, Instant};

use crate::ids::{NodeId, RunId, WorkerId};
use crate::log::{Durable, Log, Logged, MemoryLog, Record, RunEstate};
use crate::message::{BranchId, Message, Report};
use crate::routes::Routes;
use crate::run::RunOutcome;
use crate::tasks::{Kind, Task, TaskRef, Tasks};
use crate::transport::{LocalTransport, MeshTls, TcpTransport, Transport};
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
    running: Arc<Mutex<HashMap<RunId, (WorkerId, Started)>>>,
    /// The reports each run's owner sent as it recorded them, by branch, as the control plane
    /// keeps them: one sent again counts once. A run's outcome is made of them.
    recorded: Arc<Mutex<Recorded>>,
    /// Workers draining: still running, but given no new work.
    leaving: Mutex<HashSet<WorkerId>>,
    dead: Arc<Dead>,
    successors: Mutex<HashMap<WorkerId, WorkerId>>,
    tasks: Mutex<BTreeSet<TaskRef>>,
    next_owner: AtomicUsize,
    log: Arc<MemoryLog>,
}

type Recorded = HashMap<RunId, BTreeMap<BranchId, Report>>;

/// A run as it was started: what its owner, or the owner's successor, starts it with.
#[derive(Clone)]
struct Started {
    workflow: Workflow,
    inputs: Vec<(ValueId, Value)>,
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
        Self::with_log_delay(task_sets, Duration::ZERO)
    }

    /// Like [`LocalCluster::new`], but what a worker logs becomes durable only `delay` later, in
    /// order, as with a log written behind the critical path over a network (Redis): a worker
    /// killed in that time loses what it had not yet made durable.
    pub fn with_log_delay(task_sets: Vec<Arc<dyn Tasks>>, delay: Duration) -> Self {
        let transport = Arc::new(LocalTransport::default());
        let mut connect = |id: WorkerId| -> Endpoint {
            let inbox = transport.connect(id);
            (transport.clone(), inbox)
        };
        let control = connect(CONTROL);
        let endpoints = (1..=task_sets.len() as WorkerId)
            .map(&mut connect)
            .collect();
        Self::assemble(task_sets, control, endpoints, delay)
    }

    /// Like [`LocalCluster::new`], but the workers talk over TCP on localhost, so every message
    /// is encoded, sent and decoded exactly as between machines.
    pub async fn tcp(task_sets: Vec<Arc<dyn Tasks>>) -> std::io::Result<Self> {
        Self::tcp_with(task_sets, None).await
    }

    /// Like [`LocalCluster::tcp`], with mutual TLS between every member if `tls` is given.
    pub async fn tcp_with(
        task_sets: Vec<Arc<dyn Tasks>>,
        tls: Option<MeshTls>,
    ) -> std::io::Result<Self> {
        let mut transports = Vec::new();
        for _ in 0..=task_sets.len() {
            let localhost = ([127, 0, 0, 1], 0).into();
            transports.push(TcpTransport::bind(localhost, tls.clone()).await?);
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
        Ok(Self::assemble(
            task_sets,
            control,
            endpoints,
            Duration::ZERO,
        ))
    }

    /// Starts `n` workers that all serve `tasks`.
    pub fn uniform(n: usize, tasks: Arc<dyn Tasks>) -> Self {
        Self::new(vec![tasks; n])
    }

    fn assemble(
        task_sets: Vec<Arc<dyn Tasks>>,
        control: Endpoint,
        endpoints: Vec<Endpoint>,
        log_delay: Duration,
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
                let log = Arc::new(MortalLog::new(id, log.clone(), dead.clone(), log_delay));
                let slots = Worker::default_task_slots();
                Worker::spawn(id, tasks, routes.clone(), transport, log, inbox, slots)
            })
            .collect();
        routes.set_workers(workers.iter().map(Worker::id).collect());

        let waiting: Arc<Mutex<HashMap<RunId, oneshot::Sender<RunOutcome>>>> = Arc::default();
        let running: Arc<Mutex<HashMap<RunId, (WorkerId, Started)>>> = Arc::default();
        let recorded: Arc<Mutex<Recorded>> = Arc::default();
        let (control, mut inbox) = control;
        let (finished, over, reports) = (waiting.clone(), running.clone(), recorded.clone());
        let replies = control.clone();
        // The cluster starts its runs, as the control plane does: it keeps their reports, each
        // branch's once, tells their owners they are stored, and gives each run's outcome.
        tokio::spawn(async move {
            while let Some(message) = inbox.recv().await {
                match message {
                    Message::Recorded {
                        run,
                        branch,
                        report,
                        delivery,
                    } => {
                        let mut reports = reports.lock().unwrap();
                        reports
                            .entry(run)
                            .or_default()
                            .entry(branch)
                            .or_insert(report);
                        drop(reports);
                        let seqs = vec![delivery.seq];
                        replies.send(delivery.from, Message::Stored { seqs });
                    }
                    Message::Finished { run, workers } => {
                        over.lock().unwrap().remove(&run);
                        let mut outcome = RunOutcome {
                            workers,
                            ..RunOutcome::default()
                        };
                        let reports = reports.lock().unwrap();
                        for report in reports.get(&run).into_iter().flat_map(BTreeMap::values) {
                            match report.clone() {
                                Report::Result(value) => outcome.results.push(value),
                                Report::Failure(failure) => outcome.failures.push(failure),
                            }
                        }
                        drop(reports);
                        if let Some(waiter) = finished.lock().unwrap().remove(&run) {
                            let _ = waiter.send(outcome);
                        }
                    }
                    _ => {}
                }
            }
        });
        Self {
            workers,
            routes,
            control,
            waiting,
            running,
            recorded,
            dead,
            successors: Mutex::default(),
            leaving: Mutex::default(),
            tasks: Mutex::default(),
            next_owner: AtomicUsize::new(0),
            log,
        }
    }

    /// The reports of every run so far, as their owners sent them while the runs went: each
    /// branch's once, however often it was sent, in branch order.
    pub fn recorded(&self) -> HashMap<RunId, Vec<Report>> {
        let recorded = self.recorded.lock().unwrap();
        recorded
            .iter()
            .map(|(&run, reports)| (run, reports.values().cloned().collect()))
            .collect()
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

    /// Runs `workflow`, which takes no inputs, to its end.
    pub async fn run(&self, workflow: Workflow) -> anyhow::Result<RunOutcome> {
        self.run_with(workflow, Inputs::new()).await
    }

    /// Runs `workflow` with `inputs` to its end. Inputs that do not match the workflow's
    /// parameters are refused before anything runs.
    pub async fn run_with(&self, workflow: Workflow, inputs: Inputs) -> anyhow::Result<RunOutcome> {
        let inputs = inputs.bind(&workflow)?;
        let tasks = workflow
            .operations
            .iter()
            .filter_map(Operation::task_id)
            .map(|id| TaskRef {
                id: id.to_string(),
                version: String::new(),
            });
        self.tasks.lock().unwrap().extend(tasks);
        self.reroute()?;
        self.check(&workflow)?;
        let workflow = self.share(&workflow)?;

        let live = self.live();
        let i = self.next_owner.fetch_add(1, Ordering::Relaxed);
        let owner = live[i % live.len()].id();
        let run = RunId::new_v4();
        let (done, outcome) = oneshot::channel();
        self.waiting.lock().unwrap().insert(run, done);
        let started = Started { workflow, inputs };
        self.running
            .lock()
            .unwrap()
            .insert(run, (owner, started.clone()));
        self.start(owner, run, started);
        outcome.await.context("run abandoned")
    }

    /// Checks `workflow`'s calls against the signatures of the tasks the workers serve, as the
    /// orchestrator does when a workflow is published. A call may be routed to any worker that
    /// serves its task, so it must fit each one's.
    fn check(&self, workflow: &Workflow) -> anyhow::Result<()> {
        let tasks: BTreeSet<&str> = workflow
            .operations
            .iter()
            .filter_map(Operation::task_id)
            .collect();
        let mut problems = BTreeSet::new();
        for worker in self.live() {
            let signatures: HashMap<&str, Signature> = tasks
                .iter()
                .filter_map(|&id| {
                    let task = TaskRef {
                        id: id.to_string(),
                        version: String::new(),
                    };
                    Some((id, worker.signature(&task)?))
                })
                .collect();
            let report = namu_core::signature::check(workflow, |task| signatures.get(task));
            problems.extend(report.problems);
        }
        anyhow::ensure!(
            problems.is_empty(),
            "workflow {} does not fit its tasks:\n{}",
            workflow.name,
            problems.into_iter().collect::<Vec<_>>().join("\n")
        );
        Ok(())
    }

    /// `workflow` with what every branch of a scope computes alike moved out of it (design
    /// §6.1), as the orchestrator does for the runs of a published workflow. A task's calls are
    /// shared only if every worker that serves it may share them.
    fn share(&self, workflow: &Workflow) -> anyhow::Result<Workflow> {
        let live = self.live();
        namu_core::hoist::hoist(workflow, |id| {
            let task = TaskRef {
                id: id.to_string(),
                version: String::new(),
            };
            let served: Vec<Task> = live
                .iter()
                .filter_map(|worker| worker.task(&task))
                .collect();
            namu_core::hoist::Task {
                shareable: !served.is_empty() && served.iter().all(Task::shareable),
                stream: served
                    .iter()
                    .any(|task| matches!(task.kind, Kind::Stream(_))),
            }
        })
    }

    fn start(&self, owner: WorkerId, run: RunId, started: Started) {
        let Started { workflow, inputs } = started;
        let start = Message::Start {
            run,
            workflow,
            inputs,
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
        let orphaned: Vec<(RunId, Started)> = self
            .running
            .lock()
            .unwrap()
            .iter_mut()
            .filter(|(_, (owner, _))| *owner == worker)
            .map(|(&run, (owner, started))| {
                *owner = successor;
                (run, started.clone())
            })
            .collect();
        for (run, started) in orphaned {
            self.start(successor, run, started);
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

/// A worker's view of the shared log; a killed worker's appends never land, nor do those still
/// on their way when it is killed.
struct MortalLog {
    worker: WorkerId,
    inner: Arc<MemoryLog>,
    dead: Arc<Dead>,
    /// Appends on their way, if they take time to become durable.
    writer: Option<mpsc::UnboundedSender<Append>>,
}

/// An append on its way: when it was made, its records, and who waits for it to be durable.
type Append = (Instant, Vec<Record>, oneshot::Sender<()>);

impl MortalLog {
    fn new(worker: WorkerId, inner: Arc<MemoryLog>, dead: Arc<Dead>, delay: Duration) -> Self {
        let writer = (!delay.is_zero()).then(|| {
            let (writer, mut appends) = mpsc::unbounded_channel::<Append>();
            let (inner, dead) = (inner.clone(), dead.clone());
            tokio::spawn(async move {
                while let Some((sent, records, durable)) = appends.recv().await {
                    tokio::time::sleep_until(sent + delay).await;
                    if dead.contains(worker) {
                        return;
                    }
                    drop(inner.append(records));
                    let _ = durable.send(());
                }
            });
            writer
        });
        Self {
            worker,
            inner,
            dead,
            writer,
        }
    }
}

#[async_trait]
impl Log for MortalLog {
    fn append(&self, records: Vec<Record>) -> Durable {
        if self.dead.contains(self.worker) {
            return oneshot::channel().1;
        }
        match &self.writer {
            Some(writer) => {
                let (durable, done) = oneshot::channel();
                let _ = writer.send((Instant::now(), records, durable));
                done
            }
            None => self.inner.append(records),
        }
    }

    async fn node(&self, run: RunId, id: NodeId) -> anyhow::Result<Option<Arc<Node>>> {
        self.inner.node(run, id).await
    }

    async fn run(&self, run: RunId) -> anyhow::Result<Logged> {
        self.inner.run(run).await
    }

    async fn estate(&self, worker: WorkerId) -> anyhow::Result<Vec<RunEstate>> {
        self.inner.estate(worker).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use namu_core::ir::{Action, Call, Literal, Next, Operation, Workflow};

    use super::*;
    use crate::FnTasks;

    /// `count(1)`: one call of `count`, which counts its calls.
    fn once() -> Workflow {
        let call = Call {
            task_id: "count".into(),
            inputs: vec![0],
            outputs: vec![1],
            catch: None,
        };
        let literal = Literal {
            output: 0,
            value: "1".into(),
        };
        let op = Operation::new(
            vec![literal],
            Vec::new(),
            Some(Action::Call(call)),
            Next::return_value(1),
        );
        Workflow::new("once".into(), vec![op])
    }

    /// A control plane that restarted before it heard a run end, or a dead owner's successor,
    /// may be sent its `Start` again: the run must not run again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_start_sent_again_after_the_run_ended_does_not_run_it_again() {
        let calls = Arc::new(AtomicUsize::new(0));
        let tasks = FnTasks::new().with("count", {
            let calls = calls.clone();
            move |x: i32| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>(x)
            }
        });
        let cluster = LocalCluster::uniform(2, Arc::new(tasks));
        cluster.run(once()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let run = *cluster.recorded().keys().next().expect("the run reported");

        for worker in [1, 2] {
            let started = Started {
                workflow: once(),
                inputs: Vec::new(),
            };
            cluster.start(worker, run, started);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the run ran again");
        assert_eq!(cluster.recorded()[&run].len(), 1);
    }
}
