//! Workers in one process: the reference configuration.
//!
//! It is the same engine that runs across machines, with channels as the transport. Every
//! workflow must produce the same results here as on any number of machines, which is what
//! makes it the standard the distributed runtime is checked against.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use namu_core::ir::Workflow;

use crate::ids::WorkerId;
use crate::routes::Routes;
use crate::run::RunOutcome;
use crate::tasks::{TaskRef, Tasks};
use crate::transport::{LocalTransport, TcpTransport};
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

    /// Like [`LocalCluster::new`], but the workers talk over TCP on localhost, so every message
    /// is encoded, sent and decoded exactly as between machines.
    pub async fn tcp(task_sets: Vec<Arc<dyn Tasks>>) -> std::io::Result<Self> {
        assert!(!task_sets.is_empty(), "a cluster needs a worker");
        let mut endpoints = Vec::new();
        for _ in &task_sets {
            endpoints.push(TcpTransport::bind(([127, 0, 0, 1], 0).into()).await?);
        }
        let ids: Vec<WorkerId> = (1..).take(endpoints.len()).collect();
        for (transport, _, _) in &endpoints {
            for (&id, (_, addr, _)) in ids.iter().zip(&endpoints) {
                transport.set_peer(id, *addr);
            }
        }
        let routes = Arc::new(Routes::default());
        routes.set_workers(ids.clone());
        let workers = task_sets
            .into_iter()
            .zip(endpoints)
            .zip(ids)
            .map(|((tasks, (transport, _, inbox)), id)| {
                Worker::spawn(id, tasks, routes.clone(), transport, inbox)
            })
            .collect();
        Ok(Self {
            workers,
            routes,
            next_owner: AtomicUsize::new(0),
        })
    }

    /// Starts `n` workers that all serve `tasks`.
    pub fn uniform(n: usize, tasks: Arc<dyn Tasks>) -> Self {
        Self::new(vec![tasks; n])
    }

    /// Runs `workflow` to completion.
    pub async fn run(&self, workflow: Workflow) -> anyhow::Result<RunOutcome> {
        let tasks: BTreeSet<TaskRef> = workflow
            .operations
            .iter()
            .filter_map(|op| op.call.as_ref())
            .map(|call| TaskRef {
                id: call.task_id.clone(),
                version: String::new(),
            })
            .collect();
        for task in tasks {
            let servers: Vec<WorkerId> = self
                .workers
                .iter()
                .filter(|w| w.serves(&task))
                .map(Worker::id)
                .collect();
            // Membership is fixed here, so a branch waiting for a server would wait forever.
            anyhow::ensure!(!servers.is_empty(), "no worker serves task {}", task.id);
            self.routes.set_servers(task, servers);
        }

        let i = self.next_owner.fetch_add(1, Ordering::Relaxed);
        let owner = &self.workers[i % self.workers.len()];
        owner.run(workflow, BTreeMap::new()).await
    }
}
