//! Which workers exist, which can run which task, and who took over for the dead, as published
//! by the control plane; and how busy each worker is, as the workers report it (design §10.3).
//!
//! Work goes to the least loaded worker that can take it. A worker's load is what it last
//! reported (on the acknowledgements it sends anyway) plus what was sent to it since, per task
//! slot. A worker that has not reported counts as idle, with as many slots as this machine has
//! (the likeliest guess in a cluster of alike machines; its first report corrects it), so a new
//! worker draws work at once, and not only a share of one slot's worth.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, RwLock};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::ids::WorkerId;
use crate::tasks::TaskRef;

#[derive(Debug)]
pub struct Routes {
    workers: RwLock<Vec<WorkerId>>,
    servers: RwLock<HashMap<TaskRef, Vec<WorkerId>>>,
    /// Dead worker -> the worker that took over its work (design §7.4).
    successors: RwLock<HashMap<WorkerId, WorkerId>>,
    /// Where to start looking, so workers equally loaded take turns.
    cursor: AtomicUsize,
    loads: Mutex<HashMap<WorkerId, Estimate>>,
    /// The slots of a worker that has not reported: this machine's.
    slots: u32,
    /// Bumped on every change.
    version: watch::Sender<u64>,
}

/// How busy a worker is: branches being driven there, and how many task calls it runs at once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Load {
    pub pending: u32,
    pub slots: u32,
}

/// A worker's last reported load, if it has reported, and the work sent to it since.
#[derive(Debug, Default)]
struct Estimate {
    load: Option<Load>,
    sent: u32,
}

impl Estimate {
    /// Pending work per slot, as a fraction to compare exactly. Before its first report, a
    /// worker is taken to have `slots`.
    fn busy(&self, slots: u32) -> (u64, u64) {
        let load = self.load.unwrap_or(Load { pending: 0, slots });
        let pending = u64::from(load.pending) + u64::from(self.sent);
        (pending, u64::from(load.slots.max(1)))
    }
}

impl Default for Routes {
    fn default() -> Self {
        Self {
            workers: RwLock::default(),
            servers: RwLock::default(),
            successors: RwLock::default(),
            cursor: AtomicUsize::new(0),
            loads: Mutex::default(),
            slots: std::thread::available_parallelism().map_or(4, |n| n.get() as u32),
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

    /// A worker for work that any worker can do (a new branch), other than `skip`.
    pub fn any(&self, skip: Option<WorkerId>) -> Option<WorkerId> {
        self.pick(&self.workers.read().unwrap(), skip)
    }

    /// A worker that can run `task`, other than `skip`.
    pub fn server(&self, task: &TaskRef, skip: Option<WorkerId>) -> Option<WorkerId> {
        self.servers
            .read()
            .unwrap()
            .get(task)
            .and_then(|workers| self.pick(workers, skip))
    }

    /// A worker other than `skip` that can run `task`, waiting for one to join if there is none.
    pub async fn wait_for_server(&self, task: &TaskRef, skip: Option<WorkerId>) -> WorkerId {
        let mut changes = self.watch();
        loop {
            changes.borrow_and_update();
            if let Some(worker) = self.server(task, skip) {
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

    /// `worker` reports its load.
    pub fn report(&self, worker: WorkerId, load: Load) {
        let mut loads = self.loads.lock().unwrap();
        *loads.entry(worker).or_default() = Estimate {
            load: Some(load),
            sent: 0,
        };
    }

    /// The least loaded of `workers` but `skip`, which is then counted as one call busier.
    fn pick(&self, workers: &[WorkerId], skip: Option<WorkerId>) -> Option<WorkerId> {
        let workers: Vec<WorkerId> = workers
            .iter()
            .copied()
            .filter(|&worker| Some(worker) != skip)
            .collect();
        if workers.is_empty() {
            return None;
        }
        let start = self.cursor.fetch_add(1, Ordering::Relaxed);
        let mut loads = self.loads.lock().unwrap();
        let busy = |worker: &WorkerId| {
            loads
                .get(worker)
                .map_or((0, 1), |estimate| estimate.busy(self.slots))
        };
        let mut best = workers[start % workers.len()];
        for offset in 1..workers.len() {
            let worker = workers[(start + offset) % workers.len()];
            let ((a, a_slots), (b, b_slots)) = (busy(&worker), busy(&best));
            if a * b_slots < b * a_slots {
                best = worker;
            }
        }
        loads.entry(best).or_default().sent += 1;
        Some(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_goes_to_the_least_loaded_per_slot() {
        let routes = Routes::default();
        routes.set_workers(vec![1, 2, 3]);
        routes.report(
            1,
            Load {
                pending: 8,
                slots: 4,
            },
        );
        routes.report(
            2,
            Load {
                pending: 3,
                slots: 1,
            },
        );
        routes.report(
            3,
            Load {
                pending: 1,
                slots: 2,
            },
        );
        // 3 is at 0.5 per slot; once sent two calls it is at 1.5, and 1 (at 2) is still busier.
        let picks: Vec<_> = (0..3).map(|_| routes.any(None).unwrap()).collect();
        assert_eq!(picks, [3, 3, 3]);
        // At 2 per slot each now, they take turns; a report starts the count again.
        routes.report(
            3,
            Load {
                pending: 0,
                slots: 2,
            },
        );
        assert_eq!(routes.any(None), Some(3));
    }

    #[test]
    fn a_worker_that_has_not_reported_draws_work() {
        let routes = Routes::default();
        routes.set_workers(vec![1, 2]);
        routes.report(
            1,
            Load {
                pending: 5,
                slots: 4,
            },
        );
        assert_eq!(routes.any(None), Some(2));
    }

    #[test]
    fn a_worker_that_has_not_reported_counts_with_a_machine_s_slots() {
        // One worker reported 14 slots; two have not. Work sent to them is weighed per slot as
        // for the first, so it spreads instead of piling onto the one that reported.
        let routes = Routes {
            slots: 14,
            ..Routes::default()
        };
        routes.set_workers(vec![1, 2, 3]);
        routes.report(
            1,
            Load {
                pending: 0,
                slots: 14,
            },
        );
        let mut picks = HashMap::new();
        for _ in 0..42 {
            *picks.entry(routes.any(None).unwrap()).or_insert(0) += 1;
        }
        assert_eq!(picks, HashMap::from([(1, 14), (2, 14), (3, 14)]));
    }

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
