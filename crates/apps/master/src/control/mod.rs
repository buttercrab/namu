//! The control plane's rules, apart from HTTP and from where state is kept.
//!
//! - **Registry.** Task and workflow versions are immutable: lineage recovery runs again the exact
//!   version a run used. Publishing a version again is fine if it is identical, and a conflict
//!   otherwise. A workflow is published once every task it calls is.
//! - **Runs.** A run starts once its inputs are checked; a worker that is not draining owns it,
//!   chosen in turn. Workers drive it; its owner reports how it ended.
//! - **Membership.** A worker joins with a lease it renews by heartbeats. One whose lease ran out
//!   is dead: the lowest live worker succeeds it, taking over what it held from the log, and the
//!   runs it owned before they reached the log are started again on the successor.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use bytes::Bytes;
use namu_core::inputs::Inputs;
use namu_core::ir::Workflow;
use namu_core::value::Value;
use namu_engine::WorkerStats;
use namu_engine::message::{BranchId, Delivery, Message, Report};
use namu_engine::transport::{TcpTransport, Transport};
use namu_proto::{
    MEMBERSHIP_TTL_MS, Member, Membership, Page, PageQuery, Published, RunCreateRequest, RunStatus,
    RunStatusResponse, TaskKind, TaskManifest, WorkerRegisterRequest, WorkerRegisterResponse,
    WorkflowUploadRequest,
};
use serde_json::Value as Json;
use tokio::time::timeout_at;
use uuid::Uuid;

use crate::artifacts::{self, Artifacts};
use crate::error::ApiError;
use crate::store::{Inserted, Reported, RunEnd, RunRecord, RunSpec, Store};

/// The control plane's link to the workers: it is peer 0 of their mesh.
pub trait Mesh: Send + Sync + 'static {
    fn connect(&self, worker: u64, addr: SocketAddr);
    fn disconnect(&self, worker: u64);
    fn send(&self, worker: u64, message: Message);
}

impl Mesh for TcpTransport {
    fn connect(&self, worker: u64, addr: SocketAddr) {
        self.set_peer(worker, addr);
    }

    fn disconnect(&self, worker: u64) {
        self.remove_peer(worker);
    }

    fn send(&self, worker: u64, message: Message) {
        Transport::send(self, worker, message);
    }
}

const LEASE: Duration = Duration::from_millis(MEMBERSHIP_TTL_MS);

/// A published workflow version, as runs start it: with what every branch of a scope computes
/// alike moved out of the scope (design §6.1), decided by its tasks' manifests, which never
/// change either.
struct Version {
    workflow: Workflow,
    task_versions: BTreeMap<String, String>,
}

pub struct Control {
    store: Arc<dyn Store>,
    artifacts: Arc<Artifacts>,
    mesh: Arc<dyn Mesh>,
    /// Where workers reach the control plane in the mesh.
    mesh_addr: String,
    /// Workflow versions runs started, parsed once: they never change.
    versions: Mutex<HashMap<(String, String), Arc<Version>>>,
    /// This incarnation of the control plane (see `Membership::epoch`).
    epoch: u64,
    /// Whether the last sweep failed: the store was unreachable, so nobody could renew a lease.
    impaired: std::sync::atomic::AtomicBool,
    /// One sweep at a time: a worker's leaving sweeps too, besides the periodic sweep.
    sweeping: tokio::sync::Mutex<()>,
    /// Moves on whenever a report is stored or a run ends, for pages that wait for one.
    reported: tokio::sync::watch::Sender<u64>,
}

impl Control {
    pub fn new(
        store: Arc<dyn Store>,
        artifacts: Arc<Artifacts>,
        mesh: Arc<dyn Mesh>,
        mesh_addr: String,
    ) -> Self {
        Self {
            store,
            artifacts,
            mesh,
            mesh_addr,
            versions: Mutex::default(),
            epoch: Uuid::new_v4().as_u64_pair().0,
            impaired: std::sync::atomic::AtomicBool::new(false),
            sweeping: tokio::sync::Mutex::new(()),
            reported: tokio::sync::watch::Sender::new(0),
        }
    }

    pub async fn publish_task(&self, artifact: Bytes) -> Result<Published, ApiError> {
        let mut manifest = artifacts::manifest_of(&artifact).map_err(ApiError::bad_request)?;
        check_id("task", &manifest.task_id, &manifest.version)?;
        manifest.created_at = now_ms().to_string();
        let (id, version) = (manifest.task_id.clone(), manifest.version.clone());
        let unchanged = match self.store.insert_task(&manifest).await? {
            Inserted::New => false,
            Inserted::Existing(existing) if same_task(&existing, &manifest) => true,
            Inserted::Existing(_) => {
                return Err(ApiError::Conflict(format!(
                    "task {id}@{version} is published with a different artifact or manifest; \
                     bump the version"
                )));
            }
        };
        // The version is decided once the manifest is stored; its artifact follows. Publishing it
        // again, identically, writes an artifact a crash in between left out.
        if !unchanged || self.artifacts.get(&id, &version).await?.is_none() {
            self.artifacts.put(&id, &version, artifact).await?;
        }
        Ok(Published {
            id,
            version,
            unchanged,
            unchecked: Vec::new(),
        })
    }

    pub async fn task(&self, id: &str, version: &str) -> Result<TaskManifest, ApiError> {
        self.store
            .task(id, version)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("no task {id}@{version}")))
    }

    pub async fn artifact(&self, id: &str, version: &str) -> Result<Bytes, ApiError> {
        check_id("task", id, version)?;
        self.artifacts
            .get(id, version)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("no artifact for task {id}@{version}")))
    }

    pub async fn publish_workflow(
        &self,
        upload: WorkflowUploadRequest,
    ) -> Result<Published, ApiError> {
        check_id("workflow", &upload.id, &upload.version)?;
        let workflow: Workflow =
            serde_json::from_value(upload.ir.clone()).map_err(ApiError::bad_request)?;
        // The analyses every worker runs on it, run once here, so a broken workflow is refused
        // rather than failing its runs.
        namu_core::scopes::collect_ops(&workflow).map_err(ApiError::bad_request)?;
        let called: BTreeSet<&str> = workflow
            .operations
            .iter()
            .filter_map(|op| op.task_id())
            .collect();
        let mut tasks = BTreeMap::new();
        for task in called {
            let version = upload.task_versions.get(task).ok_or_else(|| {
                ApiError::bad_request(format!("no version is given for task {task}"))
            })?;
            let Some(manifest) = self.store.task(task, version).await? else {
                return Err(ApiError::bad_request(format!(
                    "task {task}@{version} is not published"
                )));
            };
            tasks.insert(task, manifest);
        }
        // A workflow pins its tasks' versions, so checked here, its calls fit in every run.
        let report = namu_core::signature::check(&workflow, |task| {
            tasks
                .get(task)
                .and_then(|manifest| manifest.signature.as_ref())
        });
        if !report.problems.is_empty() {
            return Err(ApiError::bad_request(format!(
                "workflow {}@{} does not fit its tasks:\n{}",
                upload.id,
                upload.version,
                report.problems.join("\n")
            )));
        }
        let (id, version) = (upload.id.clone(), upload.version.clone());
        let unchanged = match self.store.insert_workflow(&upload).await? {
            Inserted::New => false,
            Inserted::Existing(existing)
                if existing.ir == upload.ir && existing.task_versions == upload.task_versions =>
            {
                true
            }
            Inserted::Existing(_) => {
                return Err(ApiError::Conflict(format!(
                    "workflow {id}@{version} is published with different content; bump the version"
                )));
            }
        };
        Ok(Published {
            id,
            version,
            unchanged,
            unchecked: report.unchecked.into_iter().collect(),
        })
    }

    /// Workflow version `id@version`, if it is published, parsed once.
    async fn version(&self, id: &str, version: &str) -> anyhow::Result<Option<Arc<Version>>> {
        let key = (id.to_string(), version.to_string());
        if let Some(found) = self.versions.lock().unwrap().get(&key) {
            return Ok(Some(found.clone()));
        }
        let Some(upload) = self.store.workflow(id, version).await? else {
            return Ok(None);
        };
        let workflow: Workflow = serde_json::from_value(upload.ir)
            .with_context(|| format!("workflow {id}@{version} as stored"))?;
        let mut tasks = HashMap::new();
        for (task, task_version) in &upload.task_versions {
            if let Some(manifest) = self.store.task(task, task_version).await? {
                let known = namu_core::hoist::Task {
                    shareable: manifest.shareable(),
                    stream: manifest.task_kind == TaskKind::Stream,
                };
                tasks.insert(task.clone(), known);
            }
        }
        let parsed = Arc::new(Version {
            workflow: namu_core::hoist::hoist(&workflow, |task| {
                tasks.get(task).copied().unwrap_or_default()
            })
            .with_context(|| format!("workflow {id}@{version} as stored"))?,
            task_versions: upload.task_versions.into_iter().collect(),
        });
        self.versions.lock().unwrap().insert(key, parsed.clone());
        Ok(Some(parsed))
    }

    /// Starts a run: checks its inputs, picks its owner, and sends it the workflow. From there
    /// the workers drive it.
    pub async fn start_run(&self, request: RunCreateRequest) -> Result<Uuid, ApiError> {
        let RunCreateRequest {
            workflow_id,
            version,
            inputs,
        } = request;
        let published = self
            .version(&workflow_id, &version)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("no workflow {workflow_id}@{version}")))?;
        Inputs::from_json(inputs.clone())
            .bind(&published.workflow)
            .map_err(ApiError::bad_request)?;
        let workers: Vec<Member> = self
            .store
            .leased()
            .await?
            .into_iter()
            .filter(|member| !member.draining)
            .collect();
        if workers.is_empty() {
            return Err(ApiError::Unavailable(
                "no worker can take a run: none has joined, or all are draining".to_string(),
            ));
        }
        let turn = self.store.next_owner().await?;
        let owner = &workers[(turn % workers.len() as u64) as usize];
        let id = Uuid::new_v4();
        let spec = RunSpec {
            workflow_id,
            version,
            inputs,
            created_at_ms: now_ms(),
        };
        self.store.create_run(id, &spec, owner.id).await?;
        metrics::counter!("namu_runs_started_total").increment(1);
        self.send_start(id, &spec, owner, &published)?;
        Ok(id)
    }

    fn send_start(
        &self,
        run: Uuid,
        spec: &RunSpec,
        owner: &Member,
        published: &Version,
    ) -> anyhow::Result<()> {
        let inputs = Inputs::from_json(spec.inputs.clone()).bind(&published.workflow)?;
        let addr = owner
            .addr
            .parse()
            .with_context(|| format!("worker {}'s address {:?}", owner.id, owner.addr))?;
        self.mesh.connect(owner.id, addr);
        self.mesh.send(
            owner.id,
            Message::Start {
                run,
                workflow: published.workflow.clone(),
                inputs,
                task_versions: published.task_versions.clone(),
                reply_to: 0,
            },
        );
        Ok(())
    }

    /// Branch `branch` of run `run` reported `report`, which its owner has recorded, and keeps
    /// until it is stored here. Once it is, the owner is told (`delivery`), and lets go of it. One
    /// sent again (a successor sends what it finds in the log) counts once; one that arrives
    /// after the run ended was stored before it did.
    pub async fn recorded(
        &self,
        run: Uuid,
        branch: BranchId,
        report: Report,
        delivery: Delivery,
    ) -> anyhow::Result<()> {
        let running = self
            .store
            .run(run)
            .await?
            .is_some_and(|run| run.end.is_none());
        if running {
            let report = match report {
                Report::Result(value) => Reported::Result(to_json(&value)?),
                Report::Failure(failure) => Reported::Failure(failure),
            };
            let branch = format!("{}:{}", branch.scope, branch.index);
            if self.store.record(run, &branch, &report).await? {
                let kind = match report {
                    Reported::Result(_) => "result",
                    Reported::Failure(_) => "failure",
                };
                metrics::counter!("namu_reports_stored_total", "kind" => kind).increment(1);
                self.reported.send_modify(|n| *n += 1);
            }
        }
        let seqs = vec![delivery.seq];
        self.mesh.send(delivery.from, Message::Stored { seqs });
        Ok(())
    }

    /// Run `run` is over, and `workers` is what each worker did for it. Its reports came before,
    /// each kept by its owner until it was stored here, so they are all here.
    pub async fn finished(
        &self,
        run: Uuid,
        workers: BTreeMap<u64, WorkerStats>,
    ) -> anyhow::Result<()> {
        let ids: Vec<u64> = workers.keys().copied().collect();
        let names = self.store.worker_names(&ids).await?;
        let mut calls_by_worker = BTreeMap::new();
        let mut remote_node_fetches_by_worker = BTreeMap::new();
        for ((&id, stats), name) in workers.iter().zip(names) {
            let name = name.unwrap_or_else(|| format!("worker-{id}"));
            calls_by_worker.insert(name.clone(), stats.calls);
            remote_node_fetches_by_worker.insert(name, stats.fetches);
        }
        let record = self.store.run(run).await?;
        let failed = record.as_ref().is_some_and(|run| run.failures > 0);
        let end = RunEnd {
            status: match failed {
                false => RunStatus::Succeeded,
                true => RunStatus::PartialFailed,
            },
            finished_at_ms: now_ms(),
            calls_by_worker,
            remote_node_fetches_by_worker,
        };
        self.store.end_run(run, &end).await?;
        self.reported.send_modify(|n| *n += 1);
        let status = match end.status {
            RunStatus::Succeeded => "succeeded",
            RunStatus::PartialFailed => "partial_failed",
            RunStatus::Running => "running",
        };
        metrics::counter!("namu_runs_ended_total", "status" => status).increment(1);
        if let Some(record) = record {
            trace_run(run, &record.spec, &end, status);
        }
        Ok(())
    }

    pub async fn run_status(&self, run: Uuid) -> Result<RunStatusResponse, ApiError> {
        let record = self.run(run).await?;
        let started = record.spec.created_at_ms;
        let (status, elapsed_ms, calls_by_worker, remote_node_fetches_by_worker) = match record.end
        {
            Some(end) => (
                end.status,
                end.finished_at_ms.saturating_sub(started),
                end.calls_by_worker,
                end.remote_node_fetches_by_worker,
            ),
            None => (
                RunStatus::Running,
                now_ms().saturating_sub(started),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
        };
        Ok(RunStatusResponse {
            status,
            results: record.results,
            failures: record.failures,
            elapsed_ms: Some(elapsed_ms),
            calls_by_worker,
            remote_node_fetches_by_worker,
        })
    }

    /// Run `run`'s results, in the order they arrived, from `query.after` on.
    pub async fn results(&self, run: Uuid, query: PageQuery) -> Result<Page<Json>, ApiError> {
        let limit = page_limit(query);
        self.page(run, query, || self.store.results(run, query.after, limit))
            .await
    }

    /// Run `run`'s failures, in the order they arrived, from `query.after` on.
    pub async fn failures(&self, run: Uuid, query: PageQuery) -> Result<Page<String>, ApiError> {
        let limit = page_limit(query);
        self.page(run, query, || self.store.failures(run, query.after, limit))
            .await
    }

    /// A page of run `run`'s reports, as `read` reads them. One that would be empty while the run
    /// goes on waits, up to `query.wait_ms`, for a report to be stored or the run to end.
    async fn page<T, F>(
        &self,
        run: Uuid,
        query: PageQuery,
        read: impl Fn() -> F,
    ) -> Result<Page<T>, ApiError>
    where
        F: Future<Output = anyhow::Result<Vec<T>>>,
    {
        let wait = Duration::from_millis(query.wait_ms.unwrap_or(0).min(MAX_WAIT_MS));
        let deadline = tokio::time::Instant::now() + wait;
        let mut reported = self.reported.subscribe();
        loop {
            // What is stored from here on wakes the wait below.
            reported.mark_unchanged();
            let record = self.run(run).await?;
            let page = page(&record, query, read().await?);
            let waits = page.items.is_empty() && page.status == RunStatus::Running;
            if !waits || !matches!(timeout_at(deadline, reported.changed()).await, Ok(Ok(()))) {
                return Ok(page);
            }
        }
    }

    async fn run(&self, run: Uuid) -> Result<RunRecord, ApiError> {
        self.store
            .run(run)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("no run {run}")))
    }

    /// Adds a worker to the mesh and gives it its id.
    pub async fn register(
        &self,
        request: WorkerRegisterRequest,
    ) -> Result<WorkerRegisterResponse, ApiError> {
        let addr: SocketAddr = request
            .addr
            .parse()
            .map_err(|err| ApiError::bad_request(format!("address {:?}: {err}", request.addr)))?;
        let worker_id = self.store.next_worker_id().await?;
        let member = Member {
            id: worker_id,
            name: request.name,
            addr: request.addr,
            runtimes: request.runtimes,
            platform: request.platform,
            draining: false,
        };
        self.store.join(&member, LEASE).await?;
        self.mesh.connect(worker_id, addr);
        tracing::info!(worker_id, name = member.name, %addr, "worker joined");
        Ok(WorkerRegisterResponse {
            worker_id,
            membership: self.membership().await?,
        })
    }

    /// Renews a worker's lease; returns the membership.
    pub async fn heartbeat(&self, worker: u64) -> Result<Membership, ApiError> {
        if !self.store.renew(worker, LEASE).await? {
            return Err(gone(worker));
        }
        Ok(self.membership().await?)
    }

    /// Marks a worker as draining: every member stops giving it work (design §10.4).
    pub async fn drain(&self, worker: u64) -> Result<Membership, ApiError> {
        let mut member = self
            .store
            .leased()
            .await?
            .into_iter()
            .find(|member| member.id == worker)
            .ok_or_else(|| gone(worker))?;
        member.draining = true;
        if !self.store.update_member(&member, LEASE).await? {
            return Err(gone(worker));
        }
        tracing::info!(worker, "worker draining");
        Ok(self.membership().await?)
    }

    /// A drained worker leaves. Its successor takes over what it owns from the log, as for a
    /// crash, with nothing to run again.
    pub async fn leave(&self, worker: u64) -> Result<(), ApiError> {
        self.store.end_lease(worker).await?;
        tracing::info!(worker, "worker left");
        Ok(self.sweep().await?)
    }

    /// The control plane and every member, by id, and who took over from the dead.
    pub async fn membership(&self) -> anyhow::Result<Membership> {
        let orchestrator = Member {
            id: 0,
            name: "orchestrator".to_string(),
            addr: self.mesh_addr.clone(),
            runtimes: Vec::new(),
            platform: String::new(),
            draining: false,
        };
        let mut members = vec![orchestrator];
        members.extend(self.store.members().await?);
        Ok(Membership {
            epoch: self.epoch,
            members,
            successors: self.store.successors().await?,
        })
    }

    /// Takes up where a control plane that stopped left off: everything it knew is in the
    /// store, but not who its mesh peers are, nor whether the runs it started last reached their
    /// owners. It gives every member a new lease, reconnects to it, and sends every run that has
    /// not ended its `Start` again: an owner that has the run, or had it and it ended, does
    /// nothing.
    pub async fn recover(&self) -> anyhow::Result<()> {
        // Nobody could renew a lease while the control plane was down: every member gets a new
        // one. One that died meanwhile is buried a lease after this, as any other.
        for id in self.store.alive().await? {
            self.store.renew(id, LEASE).await?;
        }
        let members = self.store.members().await?;
        for member in &members {
            match member.addr.parse() {
                Ok(addr) => self.mesh.connect(member.id, addr),
                Err(err) => tracing::warn!(member.id, member.addr, "bad member address: {err}"),
            }
        }
        let successors = self.store.successors().await?;
        let mut started = 0;
        for (run, owner) in self.store.running().await? {
            let heir = succeed(&successors, owner);
            // An owner that died unnoticed is the sweep's: it names a successor and starts the
            // run there.
            let Some(member) = members.iter().find(|member| member.id == heir) else {
                continue;
            };
            let Some(record) = self.store.run(run).await? else {
                continue;
            };
            let spec = &record.spec;
            let published = self
                .version(&spec.workflow_id, &spec.version)
                .await?
                .with_context(|| format!("run {run}'s workflow is not published"))?;
            if heir != owner {
                self.store.set_owner(run, heir).await?;
            }
            self.send_start(run, spec, member, &published)?;
            started += 1;
        }
        tracing::info!(members = members.len(), runs = started, "recovered");
        Ok(())
    }

    /// Buries the workers whose lease ran out, naming each a successor, and starts again on the
    /// successor every run whose owner died (the successor ignores a start for a run it can
    /// take over from the log, or that ended).
    pub async fn sweep(&self) -> anyhow::Result<()> {
        use std::sync::atomic::Ordering;
        let _one = self.sweeping.lock().await;
        let swept = self.sweep_once().await;
        let was = self.impaired.swap(swept.is_err(), Ordering::Relaxed);
        if was && swept.is_ok() {
            tracing::info!("the store answers again");
        }
        swept
    }

    async fn sweep_once(&self) -> anyhow::Result<()> {
        // A lease runs out only if it could have been renewed: while the store did not answer,
        // no heartbeat was heard. On its return, every member gets a new lease, as on a restart.
        if self.impaired.load(std::sync::atomic::Ordering::Relaxed) {
            for id in self.store.alive().await? {
                self.store.renew(id, LEASE).await?;
            }
            return Ok(());
        }
        // Who was alive is read before whose lease holds, so a worker that joins in between is
        // judged at the next sweep, not found alive without a lease and buried as it joins.
        let alive = self.store.alive().await?;
        let members = self.store.leased().await?;
        let Some(successor) = members.iter().map(|member| member.id).min() else {
            // Nobody to take over yet; decided once a worker joins.
            return Ok(());
        };
        for dead in alive {
            if !members.iter().any(|member| member.id == dead) {
                tracing::warn!(dead, successor, "worker died; its successor takes over");
                self.store.bury(dead, successor).await?;
                self.mesh.disconnect(dead);
            }
        }
        let successors = self.store.successors().await?;
        let running = self.store.running().await?;
        metrics::gauge!("namu_members").set(members.len() as f64);
        metrics::gauge!("namu_runs_running").set(running.len() as f64);
        for (run, owner) in running {
            let heir = succeed(&successors, owner);
            if heir == owner {
                continue;
            }
            let Some(member) = members.iter().find(|member| member.id == heir) else {
                continue;
            };
            let Some(record) = self.store.run(run).await? else {
                continue;
            };
            let spec = &record.spec;
            let published = self
                .version(&spec.workflow_id, &spec.version)
                .await?
                .with_context(|| format!("run {run}'s workflow is not published"))?;
            self.store.set_owner(run, heir).await?;
            self.send_start(run, spec, member, &published)?;
        }
        Ok(())
    }
}

/// Sends run `run`'s root span, from when it was asked for to when it ended, if it is traced.
fn trace_run(run: Uuid, spec: &RunSpec, end: &RunEnd, status: &'static str) {
    use opentelemetry::KeyValue;
    let at = |ms: u64| UNIX_EPOCH + Duration::from_millis(ms);
    namu_telemetry::run_span(
        run,
        format!("run {}", spec.workflow_id),
        at(spec.created_at_ms),
        at(end.finished_at_ms),
        vec![
            KeyValue::new("namu.run", run.to_string()),
            KeyValue::new("namu.workflow", spec.workflow_id.clone()),
            KeyValue::new("namu.version", spec.version.clone()),
            KeyValue::new("namu.status", status),
        ],
    );
}

/// Who holds what `worker` held: itself if it lives, else its successor's heir.
fn succeed(successors: &BTreeMap<u64, u64>, mut worker: u64) -> u64 {
    // Each step goes to a later death, so the chain ends; the bound only guards a broken map.
    for _ in 0..=successors.len() {
        match successors.get(&worker) {
            Some(&next) => worker = next,
            None => break,
        }
    }
    worker
}

/// Whether two manifests describe the same task version: everything but when each was published.
/// Workers place and call tasks by their manifests, so any other difference is another version.
fn same_task(a: &TaskManifest, b: &TaskManifest) -> bool {
    let comparable = |manifest: &TaskManifest| {
        let mut json = serde_json::to_value(manifest).ok()?;
        json.as_object_mut()?.remove("created_at");
        Some(json)
    };
    comparable(a).is_some_and(|a| Some(a) == comparable(b))
}

/// How many results a page holds unless asked for fewer, and at most.
const PAGE: u64 = 1_000;
const MAX_PAGE: u64 = 10_000;
/// The longest a page waits for a report.
const MAX_WAIT_MS: u64 = 30_000;

fn page_limit(query: PageQuery) -> u64 {
    query.limit.unwrap_or(PAGE).min(MAX_PAGE)
}

/// A page of `items` from `query.after` on. The run's status is read before its items, so a page
/// that says it ended holds everything up to its end.
fn page<T>(record: &RunRecord, query: PageQuery, items: Vec<T>) -> Page<T> {
    let status = record
        .end
        .as_ref()
        .map_or(RunStatus::Running, |end| end.status);
    Page {
        next: query.after + items.len() as u64,
        items,
        status,
    }
}

/// The API shows values as JSON: this is the one place they become JSON.
fn to_json(value: &Value) -> anyhow::Result<Json> {
    Ok(rmp_serde::from_slice(&value.encode()?)?)
}

fn check_id(what: &str, id: &str, version: &str) -> Result<(), ApiError> {
    for part in [id, version] {
        if !artifacts::valid_id(part) {
            return Err(ApiError::bad_request(format!(
                "invalid {what} id or version {part:?}: use letters, digits, `-`, `_` and `.`"
            )));
        }
    }
    Ok(())
}

fn gone(worker: u64) -> ApiError {
    ApiError::Gone(format!(
        "worker {worker} is no longer a member; register again"
    ))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
