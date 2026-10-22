//! The tasks this worker runs, and where the tasks it does not run are served.
//!
//! When a run reaches this worker, every task it calls is looked up once: its manifest is fetched
//! from the orchestrator, and the task is loaded if its runtime is one this worker serves. Which
//! other workers serve a task follows from the manifest's runtime key and the members' runtimes,
//! and is recomputed whenever membership changes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use anyhow::Context;
use async_trait::async_trait;
use namu_engine::routes::Routes;
use namu_engine::tasks::Task;
use namu_engine::{RunSpec, TaskRef, Tasks};
use namu_proto::{Member, TaskManifest};

use crate::runtime::{Runtimes, safe_segment};

pub struct RuntimeTasks {
    http: reqwest::Client,
    orchestrator_url: String,
    runtimes: Runtimes,
    /// Runtime keys this worker serves (see `TaskManifest::runtime_key`).
    runtime_keys: HashSet<String>,
    routes: Arc<Routes>,
    members: RwLock<Vec<Member>>,
    tasks: RwLock<HashMap<TaskRef, Entry>>,
}

/// A task a run on this worker calls.
struct Entry {
    runtime_key: String,
    /// Loaded when this worker serves the task.
    task: Option<Task>,
}

impl RuntimeTasks {
    pub fn new(
        http: reqwest::Client,
        orchestrator_url: String,
        runtimes: Runtimes,
        runtime_keys: HashSet<String>,
        routes: Arc<Routes>,
    ) -> Self {
        Self {
            http,
            orchestrator_url,
            runtimes,
            runtime_keys,
            routes,
            members: RwLock::new(Vec::new()),
            tasks: RwLock::new(HashMap::new()),
        }
    }

    /// Takes the current members and reroutes every known task.
    pub fn set_members(&self, members: Vec<Member>) {
        // The orchestrator (id 0) and draining workers take no work.
        let workers = members
            .iter()
            .filter(|member| member.id != 0 && !member.draining)
            .map(|member| member.id)
            .collect();
        self.routes.set_workers(workers);
        *self.members.write().unwrap() = members;
        for (task, entry) in self.tasks.read().unwrap().iter() {
            self.route(task, &entry.runtime_key);
        }
    }

    fn route(&self, task: &TaskRef, runtime_key: &str) {
        let servers = self
            .members
            .read()
            .unwrap()
            .iter()
            .filter(|member| !member.draining)
            .filter(|member| member.runtimes.iter().any(|key| key == runtime_key))
            .map(|member| member.id)
            .collect();
        self.routes.set_servers(task.clone(), servers);
    }

    /// Looks `task` up, and loads it if this worker serves it.
    async fn entry(&self, task: &TaskRef) -> anyhow::Result<Entry> {
        let manifest = self
            .manifest(task)
            .await
            .with_context(|| format!("manifest of {}@{}", task.id, task.version))?;
        let runtime_key = manifest.runtime_key();
        if !self.runtime_keys.contains(&runtime_key) {
            return Ok(Entry {
                runtime_key,
                task: None,
            });
        }
        let started = std::time::Instant::now();
        let kind = self.runtimes.load(&manifest).await?;
        let ms = started.elapsed().as_millis() as u64;
        tracing::info!(task = task.id, ms, "task loaded");
        Ok(Entry {
            runtime_key,
            task: Some(Task {
                kind,
                checkpoint: manifest.checkpoint,
            }),
        })
    }

    async fn manifest(&self, task: &TaskRef) -> anyhow::Result<TaskManifest> {
        let url = format!(
            "{}/tasks/{}/{}",
            self.orchestrator_url,
            safe_segment(&task.id)?,
            safe_segment(&task.version)?
        );
        let manifest = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(manifest)
    }
}

#[async_trait]
impl Tasks for RuntimeTasks {
    async fn prepare(&self, run: &RunSpec) -> anyhow::Result<()> {
        let new: HashSet<&TaskRef> = {
            let known = self.tasks.read().unwrap();
            run.tasks()
                .filter(|task| !known.contains_key(*task))
                .collect()
        };
        // All at once: a run waits for its slowest task, not for the sum of them.
        let entries = futures::future::try_join_all(new.into_iter().map(|task| async move {
            let entry = self.entry(task).await?;
            anyhow::Ok((task.clone(), entry))
        }))
        .await?;
        let mut tasks = self.tasks.write().unwrap();
        for (task, entry) in entries {
            self.route(&task, &entry.runtime_key);
            tasks.insert(task, entry);
        }
        Ok(())
    }

    fn task(&self, task: &TaskRef) -> Option<Task> {
        self.tasks.read().unwrap().get(task)?.task.clone()
    }
}
