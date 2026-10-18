//! Taking over from dead workers (design §7.4).
//!
//! The control plane names one successor for each dead worker. The successor adopts the dead
//! worker's estate from the log: its runs and scopes, and the branches it held, which run again.
//! Every worker re-sends what the dead worker had not acknowledged, and reads its nodes from the
//! log. Messages that arrive for a run or scope this worker is about to take over wait as
//! orphans until it has.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use super::scopes::record_report;
use super::{Inner, Run};
use crate::ids::{WorkerId, owner};
use crate::log::{Record, ReportRecord, RunEstate, ScopeEstate};
use crate::message::{Branch, Message};
use crate::scope::{Ended, Finish, Scope, Then};

/// How often orphans are reconsidered even without a membership change.
const ORPHAN_PATIENCE: Duration = Duration::from_secs(1);

impl Inner {
    pub(super) async fn watch_members(self: Arc<Self>) {
        let mut changes = self.routes.watch();
        let mut tick = tokio::time::interval(ORPHAN_PATIENCE);
        loop {
            tokio::select! {
                changed = changes.changed() => if changed.is_err() { return },
                _ = tick.tick() => {}
            }
            self.resend_moved();
            self.refetch_from_log();
            self.reask_runs();
            let adopted = self.adopted.lock().unwrap().clone();
            for dead in self.routes.adopted_by(self.id) {
                if !adopted.contains(&dead) {
                    self.adopt(dead).await;
                }
            }
            self.reconsider_orphans().await;
        }
    }

    /// Runs whose description was asked of a worker that died are asked of its successor.
    fn reask_runs(self: &Arc<Self>) {
        let mut moved = Vec::new();
        for (&run, pending) in self.pending.lock().unwrap().iter_mut() {
            let target = self.routes.resolve(pending.owner);
            if target != pending.asked {
                pending.asked = target;
                moved.push((run, target));
            }
        }
        for (run, target) in moved {
            let reply_to = self.id;
            self.deliver(target, Message::FetchRun { run, reply_to });
        }
    }

    async fn adopt(self: &Arc<Self>, dead: WorkerId) {
        let estates = match self.log.estate(dead).await {
            Ok(estates) => estates,
            // Retried on the next tick.
            Err(err) => return tracing::error!(dead, "reading the estate failed: {err:#}"),
        };
        tracing::info!(dead, runs = estates.len(), "taking over from a dead worker");
        for estate in estates {
            self.adopt_run(dead, estate).await;
        }
        self.adopted.lock().unwrap().insert(dead);
    }

    async fn adopt_run(self: &Arc<Self>, dead: WorkerId, estate: RunEstate) {
        let Some(info) = estate.info else { return };
        let Some(run) = self.install(info).await else {
            return;
        };
        for scope in estate.scopes {
            let reports = match scope.continuation {
                None => estate.reports.as_slice(),
                Some(_) => &[],
            };
            self.adopt_scope(&run, scope, reports);
        }
        // A report whose weight may not have reached its scope: return it again. The scope's
        // owner counts it once.
        for report in estate.reports {
            self.return_weight(&run, report.scope, report.branch, report.weight, None);
        }
        for branch in estate.branches {
            self.adopt_branch(&run, dead, branch);
        }
    }

    fn adopt_scope(
        self: &Arc<Self>,
        run: &Arc<Run>,
        estate: ScopeEstate,
        reports: &[ReportRecord],
    ) {
        let id = estate.scope;
        if run.scopes.lock().unwrap().contains_key(&id)
            || run.finished.lock().unwrap().contains(&id)
        {
            return;
        }
        let then = match estate.continuation {
            Some(continuation) => Then::Continue(continuation),
            None => {
                let mut finish = Finish {
                    outcome: Default::default(),
                    reported: HashSet::new(),
                };
                for report in reports {
                    record_report(&mut finish, report.branch, report.report.clone());
                }
                Then::Finish(Box::new(finish))
            }
        };
        let mut scope = Scope::new(then);
        let mut over = false;
        for end in estate.ends {
            over |= scope.end(end.branch, end.weight, end.item) == Ended::Over;
        }
        match over {
            true => self.scope_over(run, id, scope),
            false => {
                run.scopes.lock().unwrap().insert(id, scope);
            }
        }
    }

    /// Runs a branch the dead worker held again, from where it was recorded.
    fn adopt_branch(self: &Arc<Self>, run: &Arc<Run>, dead: WorkerId, branch: Branch) {
        let left = Record::BranchGone {
            run: run.id(),
            holder: dead,
            id: branch.id,
            hop: branch.hop,
        };
        let branch = Branch {
            hop: branch.hop + 1,
            ..branch
        };
        self.log.record(vec![
            Record::Branch {
                run: run.id(),
                holder: self.id,
                branch: branch.clone(),
            },
            left,
        ]);
        self.spawn_drive(run.clone(), branch);
    }

    /// Holds a message until membership or adoption decides where it belongs.
    pub(super) fn orphan(self: &Arc<Self>, message: Message) {
        self.orphans.lock().unwrap().push(message);
    }

    async fn reconsider_orphans(self: &Arc<Self>) {
        let orphans = std::mem::take(&mut *self.orphans.lock().unwrap());
        for message in orphans {
            self.reconsider(message).await;
        }
    }

    /// Delivers an orphan where it now belongs, drops it if what it was for is over, or keeps it.
    async fn reconsider(self: &Arc<Self>, message: Message) {
        let (run, addressee) = match &message {
            Message::End { run, scope, .. } => (*run, Some(owner(*scope))),
            Message::Report { run, .. } => (*run, None),
            Message::FetchRun { run, reply_to } => {
                let (run, reply_to) = (*run, *reply_to);
                return match self.log.run(run).await {
                    Ok(Some(info)) => self.deliver(reply_to, Message::Run { info }),
                    Ok(None) => self.deliver(reply_to, Message::RunOver { run }),
                    Err(err) => {
                        tracing::error!(%run, "reading the run failed: {err:#}");
                        self.orphan(message);
                    }
                };
            }
            _ => return self.receive(message),
        };
        let info = match self.run_state(run) {
            Some(state) => state.spec.info.clone(),
            None => match self.log.run(run).await {
                Ok(Some(info)) => info,
                // The run is over.
                Ok(None) => return self.drop_late(message),
                Err(err) => {
                    tracing::error!(%run, "reading the run failed: {err:#}");
                    return self.orphan(message);
                }
            },
        };
        let addressee = addressee.unwrap_or_else(|| info.owner());
        let target = self.routes.resolve(addressee);
        if target != self.id {
            return self.deliver(target, message);
        }
        let responsible = addressee == self.id || self.adopted.lock().unwrap().contains(&addressee);
        if !responsible {
            // This worker is the successor, but has not taken over yet.
            return self.orphan(message);
        }
        if self.run_state(run).is_none() && self.install(info).await.is_none() {
            return self.drop_late(message);
        }
        let held = self.run_state(run).is_some_and(|state| match &message {
            Message::End { scope, .. } => state.scopes.lock().unwrap().contains_key(scope),
            _ => state
                .scopes
                .lock()
                .unwrap()
                .contains_key(&state.spec.info.root_scope),
        });
        match held {
            true => self.receive(message),
            // Taken over, and still not here: the scope was abandoned (its opening branch is
            // running again under a new scope) or is over.
            false => self.drop_late(message),
        }
    }
}
