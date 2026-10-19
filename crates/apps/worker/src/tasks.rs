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
        let workers = members
            .iter()
            .map(|member| member.id)
            .filter(|&id| id != 0)
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
            .filter(|member| member.runtimes.iter().any(|key| key == runtime_key))
            .map(|member| member.id)
            .collect();
        self.routes.set_servers(task.clone(), servers);
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
        let tasks: HashSet<&TaskRef> = run.tasks().collect();
        for task in tasks {
            if self.tasks.read().unwrap().contains_key(task) {
                continue;
            }
            let manifest = self
                .manifest(task)
                .await
                .with_context(|| format!("manifest of {}@{}", task.id, task.version))?;
            let runtime_key = manifest.runtime_key();
            let loaded = match self.runtime_keys.contains(&runtime_key) {
                true => Some(Task {
                    kind: self.runtimes.load(&manifest).await?,
                    checkpoint: manifest.checkpoint,
                }),
                false => None,
            };
            self.route(task, &runtime_key);
            let entry = Entry {
                runtime_key,
                task: loaded,
            };
            self.tasks.write().unwrap().insert(task.clone(), entry);
        }
        Ok(())
    }

    fn task(&self, task: &TaskRef) -> Option<Task> {
        self.tasks.read().unwrap().get(task)?.task.clone()
    }
}
