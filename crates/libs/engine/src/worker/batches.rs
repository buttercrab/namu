//! Batch sessions (design §6.6).
//!
//! A branch that calls a batch task queues its input on this worker and waits for its own result.
//! For each batch task (any run, same version) the worker keeps a session per slot, driven in
//! steps: a step admits queued items up to the session's capacity and returns those it finished,
//! each of which resumes its branch at once.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde_json::Value;
use tokio::sync::{Notify, oneshot};
use tokio::time::Instant;

use super::Inner;
use super::drive::run_task;
use crate::tasks::{Batch, BatchPolicy, BatchSession, TaskRef};

type Outcome = anyhow::Result<Result<Value, String>>;

/// The items waiting for one batch task's sessions on this worker.
#[derive(Default)]
pub(super) struct Batcher {
    queue: Mutex<VecDeque<Waiting>>,
    arrived: Notify,
}

struct Waiting {
    input: Value,
    since: Instant,
    done: oneshot::Sender<Outcome>,
}

impl Inner {
    /// Runs one item of a batch task, in a step of one of this worker's sessions for it.
    pub(super) async fn batch_call(
        self: &Arc<Self>,
        task: &TaskRef,
        batch: &Batch,
        input: Value,
    ) -> Outcome {
        let batcher = self.batcher(task, batch);
        let (done, result) = oneshot::channel();
        let waiting = Waiting {
            input,
            since: Instant::now(),
            done,
        };
        batcher.queue.lock().unwrap().push_back(waiting);
        batcher.arrived.notify_waiters();
        result.await.context("the batch session went away")?
    }

    /// The task's batcher, with its sessions started on first use.
    fn batcher(self: &Arc<Self>, task: &TaskRef, batch: &Batch) -> Arc<Batcher> {
        let mut batchers = self.batchers.lock().unwrap();
        if let Some(batcher) = batchers.get(task) {
            return batcher.clone();
        }
        let batcher = Arc::new(Batcher::default());
        for _ in 0..batch.policy.slots.max(1) {
            let (worker, task, batch, batcher) =
                (self.clone(), task.clone(), batch.clone(), batcher.clone());
            tokio::spawn(async move { worker.drive_session(task, batch, batcher).await });
        }
        batchers.insert(task.clone(), batcher.clone());
        batcher
    }

    /// Steps one session for as long as the worker lives: admits what is queued, up to capacity,
    /// and hands back what each step finished.
    async fn drive_session(self: Arc<Self>, task: TaskRef, batch: Batch, batcher: Arc<Batcher>) {
        let policy = batch.policy;
        let mut session: Option<Box<dyn BatchSession>> = None;
        let mut active: HashMap<u64, oneshot::Sender<Outcome>> = HashMap::new();
        let mut next_id = 0;
        loop {
            if active.is_empty() {
                batcher.wait_for_items(&policy).await;
            }
            let admit: Vec<(u64, Value)> = {
                let mut queue = batcher.queue.lock().unwrap();
                let room = policy.size.saturating_sub(active.len()).min(queue.len());
                queue
                    .drain(..room)
                    .map(|waiting| {
                        next_id += 1;
                        active.insert(next_id, waiting.done);
                        (next_id, waiting.input)
                    })
                    .collect()
            };
            if session.is_none() {
                match run_task(|| batch.task.clone().open()) {
                    Ok(Ok(opened)) => session = Some(opened),
                    Ok(Err(message)) => {
                        fail_all(&mut active, |_| Ok(Err(message.clone())));
                        continue;
                    }
                    Err(err) => {
                        let err = format!("{err:#}");
                        fail_all(&mut active, |_| Err(anyhow::anyhow!("{err}")));
                        continue;
                    }
                }
            }
            let step = {
                let _slot = match self.task_slots.acquire().await {
                    Ok(slot) => slot,
                    Err(_) => return,
                };
                let session = session.as_mut().expect("opened above");
                run_task(|| session.step(admit))
            };
            match step {
                Ok(finished) => {
                    for (id, output) in finished {
                        match active.remove(&id) {
                            Some(done) => {
                                let _ = done.send(Ok(output));
                            }
                            None => tracing::warn!(
                                task = task.id,
                                id,
                                "a batch step finished an unknown item"
                            ),
                        }
                    }
                }
                Err(err) => {
                    // The session is broken: everything in it fails, and the next step opens a
                    // new one.
                    let err = format!("{}: {err:#}", task.id);
                    fail_all(&mut active, |_| Err(anyhow::anyhow!("{err}")));
                    session = None;
                }
            }
            tokio::task::yield_now().await;
        }
    }
}

impl Batcher {
    /// Waits until there is something to step: an item, or with `max_wait`, a full batch or an
    /// item that has waited long enough.
    async fn wait_for_items(&self, policy: &BatchPolicy) {
        loop {
            let arrived = self.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            let (queued, oldest) = {
                let queue = self.queue.lock().unwrap();
                (queue.len(), queue.front().map(|waiting| waiting.since))
            };
            let Some(oldest) = oldest else {
                arrived.await;
                continue;
            };
            let Some(max_wait) = policy.max_wait else {
                return;
            };
            if queued >= policy.size || oldest.elapsed() >= max_wait {
                return;
            }
            tokio::select! {
                _ = arrived => {}
                _ = tokio::time::sleep_until(oldest + max_wait) => return,
            }
        }
    }
}

fn fail_all(active: &mut HashMap<u64, oneshot::Sender<Outcome>>, outcome: impl Fn(u64) -> Outcome) {
    for (id, done) in active.drain() {
        let _ = done.send(outcome(id));
    }
}
