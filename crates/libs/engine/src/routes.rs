//! Which workers exist, which can run which task, and who took over for the dead, as published
//! by the control plane.

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::watch;

use crate::ids::WorkerId;
use crate::tasks::TaskRef;

#[derive(Debug)]
pub struct Routes {
    workers: RwLock<Vec<WorkerId>>,
    servers: RwLock<HashMap<TaskRef, Vec<WorkerId>>>,
    /// Dead worker -> the worker that took over its work (design §7.4).
    successors: RwLock<HashMap<WorkerId, WorkerId>>,
    cursor: AtomicUsize,
    /// Bumped on every change.
    version: watch::Sender<u64>,
}

impl Default for Routes {
    fn default() -> Self {
        Self {
            workers: RwLock::default(),
            servers: RwLock::default(),
            successors: RwLock::default(),
            cursor: AtomicUsize::new(0),
            version: watch::Sender::new(0),
        }
    }
}

impl Routes {
    /// The live workers.
    pub fn set_workers(&self, workers: Vec<WorkerId>) {
        *self.workers.write().unwrap() = workers;
        self.changed();
    }

    pub fn set_servers(&self, task: TaskRef, workers: Vec<WorkerId>) {
        self.servers.write().unwrap().insert(task, workers);
        self.changed();
    }

    pub fn set_successors(&self, successors: HashMap<WorkerId, WorkerId>) {
        let mut current = self.successors.write().unwrap();
        if *current != successors {
            *current = successors;
            drop(current);
            self.changed();
        }
    }

    fn changed(&self) {
        self.version.send_modify(|version| *version += 1);
    }

    /// Resolves when something changed since `seen`, and returns the new version.
    pub fn watch(&self) -> watch::Receiver<u64> {
        self.version.subscribe()
    }

    pub fn workers(&self) -> Vec<WorkerId> {
        self.workers.read().unwrap().clone()
    }

    /// The worker that now does the work addressed to `worker`: itself, or its successor.
    pub fn resolve(&self, worker: WorkerId) -> WorkerId {
        let successors = self.successors.read().unwrap();
        let mut current = worker;
        // A successor may have died too; the chain ends at a live worker.
        for _ in 0..=successors.len() {
            match successors.get(&current) {
                Some(&next) => current = next,
                None => break,
            }
        }
        current
    }

    /// Every dead worker whose work `worker` has taken over.
    pub fn adopted_by(&self, worker: WorkerId) -> HashSet<WorkerId> {
        let dead: Vec<WorkerId> = self.successors.read().unwrap().keys().copied().collect();
        dead.into_iter()
            .filter(|&dead| self.resolve(dead) == worker)
            .collect()
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
        let mut changes = self.watch();
        loop {
            changes.borrow_and_update();
            if let Some(worker) = self.server(task) {
                return worker;
            }
            tracing::info!(
                task = task.id,
                version = task.version,
                "waiting for a worker that serves the task"
            );
            if changes.changed().await.is_err() {
                // Routes are never dropped while workers use them.
                std::future::pending::<()>().await;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_follows_successors_to_a_live_worker() {
        let routes = Routes::default();
        routes.set_successors(HashMap::from([(2, 3), (3, 1)]));
        assert_eq!(routes.resolve(1), 1);
        assert_eq!(routes.resolve(2), 1);
        assert_eq!(routes.resolve(3), 1);
        assert_eq!(routes.adopted_by(1), HashSet::from([2, 3]));
        assert!(routes.adopted_by(3).is_empty());
    }
}
