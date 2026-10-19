//! Owning scopes and runs: branches end here, reports are recorded here, and a scope that is over
//! continues the branch that opened it.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::delivery::Release;
use super::{Inner, Run};
use crate::ids::{RunId, ScopeId, WorkerId, owner};
use crate::kernel::Start;
use crate::log::{EndRecord, Record, ReportRecord};
use crate::message::{Branch, BranchId, Message, Report, Resume};
use crate::run::{RunOutcome, WorkerStats};
use crate::scope::{Continuation, Ended, Finish, Scope, Then};
use crate::tree::Node;

/// How long the owner of a finished run waits for every worker to confirm it forgot the run.
const FORGET_PATIENCE: Duration = Duration::from_secs(2);

/// A finished run waiting for the other workers to forget it.
pub(super) struct Ending {
    outcome: RunOutcome,
    waiting: HashSet<WorkerId>,
    reply_to: Option<WorkerId>,
}

impl Inner {
    pub(super) fn on_end(self: &Arc<Self>, message: Message) {
        let Message::End {
            run,
            branch,
            item,
            delivery,
        } = &message
        else {
            unreachable!()
        };
        let scope_id = &branch.scope;
        let Some(state) = self.run_state(*run) else {
            return self.orphan(message);
        };
        let over = {
            let mut scopes = state.scopes.lock().unwrap();
            let Some(scope) = scopes.get_mut(scope_id) else {
                drop(scopes);
                return self.unknown_scope(*scope_id, message);
            };
            let ended = scope.end(branch.index, item.clone());
            if ended != Ended::Again {
                Self::free_credit(scope);
            }
            match ended {
                Ended::Again => {
                    drop(scopes);
                    return self.ack(*delivery);
                }
                Ended::Open => None,
                Ended::Over => scopes.remove(scope_id),
            }
        };
        let end = EndRecord {
            index: branch.index,
            item: item.clone(),
        };
        let durable = self.log.append(vec![Record::End {
            run: *run,
            scope: *scope_id,
            end,
        }]);
        self.ack_when(durable, *delivery);
        if let Some(scope) = over {
            self.scope_over(&state, *scope_id, scope);
        }
    }

    /// A branch ended in a scope this worker does not hold. The scope is over (and the end a
    /// duplicate), or it belongs to a dead worker this worker is taking over.
    fn unknown_scope(self: &Arc<Self>, scope: ScopeId, message: Message) {
        match self.is_over(scope) {
            true => self.drop_late(message),
            false => self.orphan(message),
        }
    }

    /// Whether `scope`, which is not open here, is over. A scope is opened before any of its
    /// branches exist, so one this worker minted, or took over from a dead worker, and does not
    /// hold any more is over.
    pub(super) fn is_over(&self, scope: ScopeId) -> bool {
        let owner = owner(scope);
        (owner == self.id && scope < self.ids.peek())
            || self.adopted.lock().unwrap().contains(&owner)
    }

    pub(super) fn scope_over(self: &Arc<Self>, run: &Arc<Run>, id: ScopeId, scope: Scope) {
        match scope.into_parts() {
            (_, Then::Finish(finish)) => self.finish(run, *finish),
            (items, Then::Continue { continuation, ctx }) => {
                self.resume(run, id, continuation, ctx, items);
            }
        }
    }

    /// The stream feeding scope `id` has ended: the scope has `count` branches.
    pub(super) fn close(self: &Arc<Self>, run: &Arc<Run>, id: ScopeId, count: u64) {
        let over = {
            let mut scopes = run.scopes.lock().unwrap();
            let Some(scope) = scopes.get_mut(&id) else {
                // Abandoned, after its source restarted elsewhere.
                return;
            };
            match scope.close(count) {
                Ended::Over => scopes.remove(&id),
                _ => None,
            }
        };
        self.log.record(vec![Record::Closed {
            run: run.id(),
            scope: id,
            count,
        }]);
        if let Some(scope) = over {
            self.scope_over(run, id, scope);
        }
    }

    /// Continues the branch that opened scope `id` from `ctx`, now that the scope is over.
    fn resume(
        self: &Arc<Self>,
        run: &Arc<Run>,
        id: ScopeId,
        continuation: Continuation,
        ctx: Arc<Node>,
        items: Vec<Value>,
    ) {
        let Continuation {
            branch,
            hop,
            scopes,
            collect,
            ..
        } = continuation;
        let Some(op) = collect else {
            // Nothing collects this scope: the branch that opened it is done.
            let release = Release::new(
                1,
                Record::ScopeGone {
                    run: run.id(),
                    scope: id,
                },
            );
            return self.send_end(run, branch, None, Some(release));
        };
        let output = run.spec.workflow().operations[op]
            .collect
            .as_ref()
            .expect("collect ops are found by scope analysis")
            .output;
        let live = run.spec.liveness.after_action(op);
        let node = self.grow(run, &ctx, live, vec![(output, Value::Array(items))]);
        let branch = Branch {
            id: branch,
            hop: hop + 1,
            ctx: node.id(),
            resume: Resume::Kernel(Start::After { op }),
            scopes,
        };
        // The continuing branch replaces the scope in the log.
        self.log.record(vec![
            Record::Nodes {
                run: run.id(),
                nodes: vec![node.clone()],
            },
            Record::Branch {
                run: run.id(),
                holder: self.id,
                branch: branch.clone(),
            },
            Record::ScopeGone {
                run: run.id(),
                scope: id,
            },
        ]);
        self.spawn_drive(run.clone(), branch, node);
    }

    /// Ends `branch` in its scope, with its item if it reached the collect.
    pub(super) fn send_end(
        self: &Arc<Self>,
        run: &Run,
        branch: BranchId,
        item: Option<Value>,
        release: Option<Arc<Release>>,
    ) {
        let run = run.id();
        self.send_kept(
            owner(branch.scope),
            |delivery| Message::End {
                run,
                branch,
                item,
                delivery,
            },
            release,
        );
    }

    pub(super) fn on_report(self: &Arc<Self>, message: Message) {
        let Message::Report {
            run,
            branch,
            report,
            delivery,
        } = &message
        else {
            unreachable!()
        };
        let Some(state) = self.run_state(*run) else {
            return self.orphan(message);
        };
        let recorded = {
            let mut scopes = state.scopes.lock().unwrap();
            let root = scopes.get_mut(&state.spec.info.root_scope);
            let Some(Then::Finish(finish)) = root.map(|root| &mut root.then) else {
                // The run's owner is dead and this worker is (about to be) its successor.
                drop(scopes);
                return self.orphan(message);
            };
            record_report(finish, *branch, report.clone())
        };
        if !recorded {
            return self.ack(*delivery);
        }
        let record = ReportRecord {
            branch: *branch,
            report: report.clone(),
        };
        let durable = self.log.append(vec![Record::Report {
            run: *run,
            report: record,
        }]);
        let (worker, delivery, branch) = (self.clone(), *delivery, *branch);
        tokio::spawn(async move {
            if durable.await.is_ok() {
                worker.ack(delivery);
                // Recorded: the branch can now end in its scope, and let the run end. Until its
                // end is acknowledged, the report in the log stands for it.
                worker.send_end(&state, branch, None, None);
            }
        });
    }

    /// Every branch of the run has ended: have every worker forget it, then report the outcome.
    fn finish(self: &Arc<Self>, run: &Run, finish: Finish) {
        let run_id = run.id();
        self.runs.lock().unwrap().remove(&run_id);
        let mut outcome = finish.outcome;
        record_stats(&mut outcome, self.id, run.stats());
        let waiting: HashSet<WorkerId> = self
            .routes
            .workers()
            .into_iter()
            .filter(|&worker| worker != self.id)
            .collect();
        // Recorded before asking, so no reply can arrive before it is expected.
        let ending = Ending {
            outcome,
            waiting: waiting.clone(),
            reply_to: run.spec.info.reply_to,
        };
        self.ending.lock().unwrap().insert(run_id, ending);
        for worker in waiting {
            let reply_to = self.id;
            self.deliver(
                worker,
                Message::Forget {
                    run: run_id,
                    reply_to,
                },
            );
        }
        self.check_forgotten(run_id);

        let worker = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(FORGET_PATIENCE).await;
            let ending = worker.ending.lock().unwrap().remove(&run_id);
            if let Some(ending) = ending {
                tracing::warn!(%run_id, missing = ?ending.waiting, "workers did not confirm the end of the run");
                worker.complete(run_id, ending);
            }
        });
    }

    pub(super) fn on_forget(self: &Arc<Self>, run: RunId, reply_to: WorkerId) {
        self.pending.lock().unwrap().remove(&run);
        let stats = self
            .runs
            .lock()
            .unwrap()
            .remove(&run)
            .map(|run| run.stats())
            .unwrap_or_default();
        let worker = self.id;
        self.deliver(reply_to, Message::Forgotten { run, worker, stats });
    }

    pub(super) fn on_forgotten(self: &Arc<Self>, run: RunId, worker: WorkerId, stats: WorkerStats) {
        if let Some(ending) = self.ending.lock().unwrap().get_mut(&run) {
            record_stats(&mut ending.outcome, worker, stats);
            ending.waiting.remove(&worker);
        }
        self.check_forgotten(run);
    }

    fn check_forgotten(self: &Arc<Self>, run: RunId) {
        let ending = {
            let mut ending = self.ending.lock().unwrap();
            match ending.get(&run).is_some_and(|e| e.waiting.is_empty()) {
                true => ending.remove(&run),
                false => None,
            }
        };
        if let Some(ending) = ending {
            self.complete(run, ending);
        }
    }

    fn complete(self: &Arc<Self>, run: RunId, ending: Ending) {
        tracing::debug!(%run, "run finished");
        self.log.record(vec![Record::RunGone(run)]);
        if let Some(reply_to) = ending.reply_to {
            let outcome = ending.outcome;
            self.deliver(reply_to, Message::Finished { run, outcome });
        }
    }
}

/// Adds a branch's report to the outcome, unless it was recorded before.
pub(super) fn record_report(finish: &mut Finish, branch: BranchId, report: Report) -> bool {
    if !finish.reported.insert(branch) {
        return false;
    }
    match report {
        Report::Result(value) => finish.outcome.results.push(value),
        Report::Failure(failure) => finish.outcome.failures.push(failure),
    }
    true
}

/// Adds a worker's stats to an outcome, if it did anything for the run.
fn record_stats(outcome: &mut RunOutcome, worker: WorkerId, stats: WorkerStats) {
    if stats != WorkerStats::default() {
        outcome.workers.insert(worker, stats);
    }
}
