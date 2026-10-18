//! Which workers exist and which can run which task, as published by the control plane.

use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ids::WorkerId;

#[derive(Debug, Default)]
pub struct Routes {
    workers: RwLock<Vec<WorkerId>>,
    servers: RwLock<HashMap<String, Vec<WorkerId>>>,
    cursor: AtomicUsize,
}

impl Routes {
    pub fn set_workers(&self, workers: Vec<WorkerId>) {
        *self.workers.write().unwrap() = workers;
    }

    pub fn set_servers(&self, task: &str, workers: Vec<WorkerId>) {
        self.servers
            .write()
            .unwrap()
            .insert(task.to_string(), workers);
    }

    pub fn workers(&self) -> Vec<WorkerId> {
        self.workers.read().unwrap().clone()
    }

    /// A worker for work that any worker can do (a new branch).
    pub fn any(&self) -> Option<WorkerId> {
        self.pick(&self.workers.read().unwrap())
    }

    /// A worker that can run `task`.
    pub fn server(&self, task: &str) -> Option<WorkerId> {
        self.servers
            .read()
            .unwrap()
            .get(task)
            .and_then(|workers| self.pick(workers))
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
