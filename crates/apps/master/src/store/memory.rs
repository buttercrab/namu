//! The control plane's state in memory, for tests. Leases expire by the clock, as in Redis.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use namu_proto::{Member, TaskManifest, WorkflowUploadRequest};
use serde_json::Value as Json;
use uuid::Uuid;

use super::{Inserted, Reported, RunEnd, RunRecord, RunSpec, Store};

#[derive(Default)]
pub struct MemoryStore(Mutex<State>);

#[derive(Default)]
struct State {
    tasks: HashMap<(String, String), TaskManifest>,
    workflows: HashMap<(String, String), WorkflowUploadRequest>,
    worker_seq: u64,
    leases: BTreeMap<u64, (Member, Instant)>,
    alive: BTreeSet<u64>,
    successors: BTreeMap<u64, u64>,
    names: HashMap<u64, String>,
    owner_cursor: u64,
    runs: HashMap<Uuid, RunRecord>,
    reports: HashMap<Uuid, Reports>,
}

#[derive(Default)]
struct Reports {
    branches: BTreeSet<String>,
    results: Vec<Json>,
    failures: Vec<String>,
}

impl State {
    fn lease(&mut self, id: u64) -> Option<&mut (Member, Instant)> {
        if self
            .leases
            .get(&id)
            .is_some_and(|(_, until)| *until <= Instant::now())
        {
            self.leases.remove(&id);
        }
        self.leases.get_mut(&id)
    }
}

impl MemoryStore {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap()
    }
}

fn insert<T: Clone>(
    map: &mut HashMap<(String, String), T>,
    key: (String, String),
    value: &T,
) -> Inserted<T> {
    match map.get(&key) {
        Some(existing) => Inserted::Existing(existing.clone()),
        None => {
            map.insert(key, value.clone());
            Inserted::New
        }
    }
}

fn page<T: Clone>(items: &[T], after: u64, limit: u64) -> Vec<T> {
    items
        .iter()
        .skip(after as usize)
        .take(limit as usize)
        .cloned()
        .collect()
}

#[async_trait]
impl Store for MemoryStore {
    async fn insert_task(&self, manifest: &TaskManifest) -> anyhow::Result<Inserted<TaskManifest>> {
        let key = (manifest.task_id.clone(), manifest.version.clone());
        Ok(insert(&mut self.state().tasks, key, manifest))
    }

    async fn task(&self, id: &str, version: &str) -> anyhow::Result<Option<TaskManifest>> {
        let key = (id.to_string(), version.to_string());
        Ok(self.state().tasks.get(&key).cloned())
    }

    async fn insert_workflow(
        &self,
        workflow: &WorkflowUploadRequest,
    ) -> anyhow::Result<Inserted<WorkflowUploadRequest>> {
        let key = (workflow.id.clone(), workflow.version.clone());
        Ok(insert(&mut self.state().workflows, key, workflow))
    }

    async fn workflow(
        &self,
        id: &str,
        version: &str,
    ) -> anyhow::Result<Option<WorkflowUploadRequest>> {
        let key = (id.to_string(), version.to_string());
        Ok(self.state().workflows.get(&key).cloned())
    }

    async fn next_worker_id(&self) -> anyhow::Result<u64> {
        let mut state = self.state();
        state.worker_seq += 1;
        Ok(state.worker_seq)
    }

    async fn join(&self, member: &Member, ttl: Duration) -> anyhow::Result<()> {
        let mut state = self.state();
        state
            .leases
            .insert(member.id, (member.clone(), Instant::now() + ttl));
        state.names.insert(member.id, member.name.clone());
        state.alive.insert(member.id);
        Ok(())
    }

    async fn renew(&self, id: u64, ttl: Duration) -> anyhow::Result<bool> {
        Ok(match self.state().lease(id) {
            Some((_, until)) => {
                *until = Instant::now() + ttl;
                true
            }
            None => false,
        })
    }

    async fn update_member(&self, member: &Member, ttl: Duration) -> anyhow::Result<bool> {
        Ok(match self.state().lease(member.id) {
            Some(lease) => {
                *lease = (member.clone(), Instant::now() + ttl);
                true
            }
            None => false,
        })
    }

    async fn end_lease(&self, id: u64) -> anyhow::Result<()> {
        self.state().leases.remove(&id);
        Ok(())
    }

    async fn members(&self) -> anyhow::Result<Vec<Member>> {
        let mut state = self.state();
        let ids: Vec<u64> = state.alive.iter().copied().collect();
        Ok(ids
            .into_iter()
            .filter_map(|id| state.lease(id).map(|(member, _)| member.clone()))
            .collect())
    }

    async fn alive(&self) -> anyhow::Result<Vec<u64>> {
        Ok(self.state().alive.iter().copied().collect())
    }

    async fn bury(&self, dead: u64, successor: u64) -> anyhow::Result<()> {
        let mut state = self.state();
        state.alive.remove(&dead);
        state.successors.insert(dead, successor);
        Ok(())
    }

    async fn successors(&self) -> anyhow::Result<BTreeMap<u64, u64>> {
        Ok(self.state().successors.clone())
    }

    async fn worker_names(&self, ids: &[u64]) -> anyhow::Result<Vec<Option<String>>> {
        let state = self.state();
        Ok(ids.iter().map(|id| state.names.get(id).cloned()).collect())
    }

    async fn next_owner(&self) -> anyhow::Result<u64> {
        let mut state = self.state();
        state.owner_cursor += 1;
        Ok(state.owner_cursor)
    }

    async fn create_run(&self, id: Uuid, spec: &RunSpec, owner: u64) -> anyhow::Result<()> {
        let record = RunRecord {
            spec: spec.clone(),
            owner,
            results: 0,
            failures: 0,
            end: None,
        };
        self.state().runs.insert(id, record);
        Ok(())
    }

    async fn run(&self, id: Uuid) -> anyhow::Result<Option<RunRecord>> {
        let state = self.state();
        Ok(state.runs.get(&id).map(|run| {
            let reports = state.reports.get(&id);
            RunRecord {
                results: reports.map_or(0, |r| r.results.len() as u64),
                failures: reports.map_or(0, |r| r.failures.len() as u64),
                ..run.clone()
            }
        }))
    }

    async fn running(&self) -> anyhow::Result<Vec<(Uuid, u64)>> {
        Ok(self
            .state()
            .runs
            .iter()
            .filter(|(_, run)| run.end.is_none())
            .map(|(&id, run)| (id, run.owner))
            .collect())
    }

    async fn set_owner(&self, id: Uuid, owner: u64) -> anyhow::Result<()> {
        if let Some(run) = self.state().runs.get_mut(&id) {
            run.owner = owner;
        }
        Ok(())
    }

    async fn end_run(&self, id: Uuid, end: &RunEnd) -> anyhow::Result<()> {
        if let Some(run) = self.state().runs.get_mut(&id) {
            run.end = Some(end.clone());
        }
        Ok(())
    }

    async fn record(&self, id: Uuid, branch: &str, report: &Reported) -> anyhow::Result<bool> {
        let mut state = self.state();
        let reports = state.reports.entry(id).or_default();
        if !reports.branches.insert(branch.to_string()) {
            return Ok(false);
        }
        match report {
            Reported::Result(json) => reports.results.push(json.clone()),
            Reported::Failure(failure) => reports.failures.push(failure.clone()),
        }
        Ok(true)
    }

    async fn results(&self, id: Uuid, after: u64, limit: u64) -> anyhow::Result<Vec<Json>> {
        let state = self.state();
        let results = state.reports.get(&id).map_or(&[][..], |r| &r.results[..]);
        Ok(page(results, after, limit))
    }

    async fn failures(&self, id: Uuid, after: u64, limit: u64) -> anyhow::Result<Vec<String>> {
        let state = self.state();
        let failures = state.reports.get(&id).map_or(&[][..], |r| &r.failures[..]);
        Ok(page(failures, after, limit))
    }
}
