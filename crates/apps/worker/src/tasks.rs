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
use namu_engine::tasks::{DEFAULT_WINDOW, TaskStream};
use namu_engine::{RunSpec, TaskRef, Tasks};
use namu_proto::{Member, TaskKind, TaskManifest};
use serde_json::Value;

use crate::runtime::{Runner, Runtimes, safe_segment};

pub struct RuntimeTasks {
    http: reqwest::Client,
    orchestrator_url: String,
    runtimes: Runtimes,
    /// Runtime keys this worker serves (see `TaskManifest::runtime_key`).
    runtime_keys: HashSet<String>,
    routes: Arc<Routes>,
    members: RwLock<Vec<Member>>,
    tasks: RwLock<HashMap<TaskRef, Task>>,
}

struct Task {
    runtime_key: String,
    checkpoint: bool,
    stream: bool,
    window: Option<usize>,
    /// Loaded when this worker serves the task.
    runner: Option<Arc<Runner>>,
}

impl RuntimeTasks {
    fn entry(&self, task: &TaskRef, f: impl FnOnce(&Task) -> bool) -> bool {
        self.tasks.read().unwrap().get(task).is_some_and(f)
    }

    fn runner(&self, task: &TaskRef) -> anyhow::Result<Arc<Runner>> {
        self.tasks
            .read()
            .unwrap()
            .get(task)
            .and_then(|entry| entry.runner.clone())
            .with_context(|| format!("task {} is not loaded here", task.id))
    }

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
            let runner = match self.runtime_keys.contains(&runtime_key) {
                true => Some(Arc::new(self.runtimes.load(&manifest).await?)),
                false => None,
            };
            self.route(task, &runtime_key);
            let entry = Task {
                runtime_key,
                checkpoint: manifest.checkpoint,
                stream: manifest.task_kind == TaskKind::Stream,
                window: manifest.window,
                runner,
            };
            self.tasks.write().unwrap().insert(task.clone(), entry);
        }
        Ok(())
    }

    fn serves(&self, task: &TaskRef) -> bool {
        self.entry(task, |entry| entry.runner.is_some())
    }

    fn checkpoint(&self, task: &TaskRef) -> bool {
        self.entry(task, |entry| entry.checkpoint)
    }

    fn is_stream(&self, task: &TaskRef) -> bool {
        self.entry(task, |entry| entry.stream)
    }

    fn window(&self, task: &TaskRef) -> usize {
        self.tasks
            .read()
            .unwrap()
            .get(task)
            .and_then(|entry| entry.window)
            .unwrap_or(DEFAULT_WINDOW)
    }

    fn open(
        &self,
        task: &TaskRef,
        input: Value,
        _cursor: Option<String>,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>> {
        self.runner(task)?.open(&input.to_string())
    }

    fn call(&self, task: &TaskRef, input: Value) -> anyhow::Result<Result<Value, String>> {
        let runner = self.runner(task)?;
        let output = match runner.call(&input.to_string())? {
            Ok(output) => output,
            Err(message) => return Ok(Err(message)),
        };
        let output = serde_json::from_str(&output)
            .with_context(|| format!("task {} returned invalid JSON", task.id))?;
        Ok(Ok(output))
    }
}
