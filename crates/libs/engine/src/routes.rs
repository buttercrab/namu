//! Which workers exist and which can run which task, as published by the control plane.

use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Notify;

use crate::ids::WorkerId;
use crate::tasks::TaskRef;

#[derive(Debug, Default)]
pub struct Routes {
    workers: RwLock<Vec<WorkerId>>,
    servers: RwLock<HashMap<TaskRef, Vec<WorkerId>>>,
    cursor: AtomicUsize,
    changed: Notify,
}

impl Routes {
    pub fn set_workers(&self, workers: Vec<WorkerId>) {
        *self.workers.write().unwrap() = workers;
        self.changed.notify_waiters();
    }

    pub fn set_servers(&self, task: TaskRef, workers: Vec<WorkerId>) {
        self.servers.write().unwrap().insert(task, workers);
        self.changed.notify_waiters();
    }

    pub fn workers(&self) -> Vec<WorkerId> {
        self.workers.read().unwrap().clone()
    }

    /// A worker for work that any worker can do (a new branch).
    pub fn any(&self) -> Option<WorkerId> {
        self.pick(&self.workers.read().unwrap())
    }

    /// A worker that can run `task`.
    pub fn server(&self, task: &TaskRef) -> Option<WorkerId> {
        self.servers
            .read()
            .unwrap()
            .get(task)
            .and_then(|workers| self.pick(workers))
    }

    /// A worker that can run `task`, waiting for one to join if there is none.
    pub async fn wait_for_server(&self, task: &TaskRef) -> WorkerId {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            // Registered before checking, so a change in between is not missed.
            changed.as_mut().enable();
            if let Some(worker) = self.server(task) {
                return worker;
            }
            tracing::info!(
                task = task.id,
                version = task.version,
                "waiting for a worker that serves the task"
            );
            changed.await;
        }
    }

    // Round robin. Load-aware placement comes with credits (docs/design.md §10.3).
    fn pick(&self, workers: &[WorkerId]) -> Option<WorkerId> {
        if workers.is_empty() {
            return None;
        }
        let i = self.cursor.fetch_add(1, Ordering::Relaxed);
        Some(workers[i % workers.len()])
    }
}
