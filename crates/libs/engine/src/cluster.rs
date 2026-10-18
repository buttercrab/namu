//! Workers in one process: the reference configuration.
//!
//! It is the same engine that runs across machines, with channels as the transport. Every
//! workflow must produce the same results here as on any number of machines, which is what
//! makes it the standard the distributed runtime is checked against.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Context;
use namu_core::ir::Workflow;

use crate::ids::WorkerId;
use crate::routes::Routes;
use crate::run::RunOutcome;
use crate::tasks::Tasks;
use crate::transport::LocalTransport;
use crate::worker::Worker;

pub struct LocalCluster {
    workers: Vec<Worker>,
    routes: Arc<Routes>,
    next_owner: AtomicUsize,
}

impl LocalCluster {
    /// Starts one worker per task set. Must be called within a Tokio runtime.
    pub fn new(task_sets: Vec<Arc<dyn Tasks>>) -> Self {
        assert!(!task_sets.is_empty(), "a cluster needs a worker");
        let transport = Arc::new(LocalTransport::default());
        let routes = Arc::new(Routes::default());
        let workers: Vec<Worker> = task_sets
            .into_iter()
            .zip(1..)
            .map(|(tasks, id): (_, WorkerId)| {
                let inbox = transport.connect(id);
                Worker::spawn(id, tasks, routes.clone(), transport.clone(), inbox)
            })
            .collect();
        routes.set_workers(workers.iter().map(Worker::id).collect());
        Self {
            workers,
            routes,
            next_owner: AtomicUsize::new(0),
        }
    }

    /// Starts `n` workers that all serve `tasks`.
    pub fn uniform(n: usize, tasks: Arc<dyn Tasks>) -> Self {
        Self::new(vec![tasks; n])
    }

    /// Runs `workflow` to completion.
    pub async fn run(&self, workflow: Workflow) -> anyhow::Result<RunOutcome> {
        let tasks: BTreeSet<&str> = workflow
            .operations
            .iter()
            .filter_map(|op| op.call.as_ref().map(|call| call.task_id.as_str()))
            .collect();
        for task in tasks {
            let servers = self.workers.iter().filter(|w| w.serves(task));
            self.routes
                .set_servers(task, servers.map(Worker::id).collect());
        }

        let i = self.next_owner.fetch_add(1, Ordering::Relaxed);
        let owner = &self.workers[i % self.workers.len()];
        let spec = owner.new_run(workflow)?;
        for worker in &self.workers {
            worker.install(spec.clone());
        }
        owner.start(&spec)?.await.context("run abandoned")
    }
}
