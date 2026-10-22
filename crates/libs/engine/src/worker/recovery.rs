//! Taking over from dead workers (design §7.4).
//!
//! The control plane names one successor for each dead worker. The successor adopts the dead
//! worker's estate from the log: its runs and scopes, and the branches it held, which run again.
//! Every worker re-sends what the dead worker had not acknowledged, and reads its nodes from the
//! log.
//!
//! A message this worker cannot place (an orphan) is placed as soon as it can be. One for a dead
//! worker this worker is taking over waits until the takeover is done; one sent here for a
//! worker this worker still believes alive waits until membership says otherwise. Nothing waits
//! on a timer.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use namu_core::value::Value;

use super::scopes::record_report;
use super::view::View;
use super::{Inner, Run};
use crate::ids::{RunId, WorkerId, owner};
use crate::log::{Record, ReportRecord, RunEstate, ScopeEstate};
use crate::message::{Branch, End, FetchRun, Message, Reported};
use crate::run::RunInfo;
use crate::scope::{Continuation, Finish, Fold, Scope, Then};
use crate::tree::Node;

/// How long to wait before trying a failed log read again.
const RETRY: Duration = Duration::from_millis(500);

/// Dead workers this worker has taken over, and the messages waiting for a takeover or a
/// membership change. One lock covers both, so a message cannot start waiting just after what it
/// waits for has happened.
#[derive(Default)]
pub(super) struct Adoption {
    adopted: HashSet<WorkerId>,
    waiting: Vec<Message>,
}

impl Inner {
    pub(super) async fn watch_members(self: Arc<Self>) {
        let mut changes = self.routes.watch();
        while changes.changed().await.is_ok() {
            self.resend_moved();
            self.refetch_from_log();
            self.reask_runs();
            self.drop_departed();
            for dead in self.routes.adopted_by(self.id) {
                if !self.has_adopted(dead) {
                    self.adopt(dead).await;
                }
            }
            self.release_waiting();
        }
    }

    pub(super) fn has_adopted(&self, worker: WorkerId) -> bool {
        self.adoption.lock().unwrap().adopted.contains(&worker)
    }

    /// Places every waiting message again.
    fn release_waiting(self: &Arc<Self>) {
        let waiting = std::mem::take(&mut self.adoption.lock().unwrap().waiting);
        for message in waiting {
            self.orphan(message);
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
            self.deliver(target, FetchRun { run, reply_to }.into());
        }
    }

    /// Takes over `dead`'s estate, trying again until the log can be read.
    async fn adopt(self: &Arc<Self>, dead: WorkerId) {
        while let Err(err) = self.adopt_estate(dead).await {
            // What was adopted already is not adopted twice.
            tracing::error!(dead, "taking over failed, trying again: {err:#}");
            tokio::time::sleep(RETRY).await;
        }
        self.adoption.lock().unwrap().adopted.insert(dead);
        self.release_waiting();
    }

    async fn adopt_estate(self: &Arc<Self>, dead: WorkerId) -> anyhow::Result<()> {
        let estates = self.log.estate(dead).await?;
        tracing::info!(dead, runs = estates.len(), "taking over from a dead worker");
        for estate in estates {
            self.adopt_run(dead, estate).await?;
        }
        Ok(())
    }

    async fn adopt_run(self: &Arc<Self>, dead: WorkerId, estate: RunEstate) -> anyhow::Result<()> {
        let Some(info) = estate.info else {
            return Ok(());
        };
        let Some(run) = self.install(info).await else {
            return Ok(());
        };
        for scope in estate.scopes {
            let reports = match scope.continuation {
                None => estate.reports.as_slice(),
                Some(_) => &[],
            };
            self.adopt_scope(&run, scope, reports).await?;
        }
        // A reported branch whose end may not have reached its scope: end it again. The scope's
        // owner counts it once.
        for report in estate.reports {
            self.send_end(&run, report.branch, None, None);
        }
        for branch in estate.branches {
            self.adopt_branch(&run, dead, branch);
        }
        Ok(())
    }

    async fn adopt_scope(
        self: &Arc<Self>,
        run: &Arc<Run>,
        estate: ScopeEstate,
        reports: &[ReportRecord],
    ) -> anyhow::Result<()> {
        let id = estate.scope;
        if run.scopes.lock().unwrap().contains_key(&id) {
            return Ok(());
        }
        let mut fold = None;
        let then = match estate.continuation {
            Some(continuation) => {
                let ctx = self.node(run, continuation.ctx).await?;
                fold = self
                    .adopt_fold(run, &continuation, &ctx, estate.folded)
                    .await?;
                Then::Continue { continuation, ctx }
            }
            None => {
                // The root scope: this worker now collects the run's outcome.
                let mut finish = Finish::default();
                for report in reports {
                    record_report(&mut finish, report.branch, report.report.clone());
                }
                *run.finish.lock().unwrap() = Some(finish);
                Then::Finish
            }
        };
        let mut scope = Scope::new(then, estate.count);
        for end in estate.ends {
            scope.end(end.index, end.item);
        }
        // After the ends, so that the items it has folded already are dropped.
        if let Some(fold) = fold {
            scope = scope.with_fold(fold);
        }
        if let Some(source) = estate.source
            && estate.count.is_none()
        {
            self.adopt_stream(run, id, scope, source);
        } else if scope.is_over() {
            self.scope_over(run, id, scope);
            return Ok(());
        } else {
            run.open_scope(id, scope);
        }
        self.fold(run, id);
        Ok(())
    }

    /// The fold of an adopted scope, if it folds: where its last logged step left it, or from its
    /// first accumulator.
    async fn adopt_fold(
        self: &Arc<Self>,
        run: &Arc<Run>,
        continuation: &Continuation,
        ctx: &Arc<Node>,
        folded: Option<(u64, Value)>,
    ) -> anyhow::Result<Option<Fold>> {
        let view = View { worker: self, run };
        let Some(collect) = continuation.collect else {
            return Ok(None);
        };
        let Some(mut fold) = self.fold_from(&view, ctx, collect).await? else {
            return Ok(None);
        };
        if let Some((through, accumulator)) = folded {
            fold.next = through;
            fold.accumulator = accumulator;
        }
        Ok(Some(fold))
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
        self.spawn_drive_fetching(run.clone(), branch);
    }

    /// Places a message this worker could not handle where it arrived: here, at the worker now
    /// responsible for it, nowhere if what it was for is over, or waiting.
    pub(super) fn orphan(self: &Arc<Self>, message: Message) {
        let worker = self.clone();
        tokio::spawn(async move { worker.place(message).await });
    }

    async fn place(self: &Arc<Self>, message: Message) {
        let (run, addressee) = match &message {
            Message::End(End { run, branch, .. }) => (*run, Some(owner(branch.scope))),
            Message::Report(Reported { run, .. }) => (*run, None),
            Message::FetchRun(FetchRun { run, reply_to }) => {
                let (run, reply_to) = (*run, *reply_to);
                return match self.log.run(run).await {
                    Ok(Some(info)) => self.deliver(reply_to, Message::Run { info }),
                    Ok(None) => self.deliver(reply_to, Message::RunOver { run }),
                    Err(err) => self.retry(message, run, err),
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
                Err(err) => return self.retry(message, run, err),
            },
        };
        let addressee = addressee.unwrap_or_else(|| info.owner());
        match self.place_for(addressee, message) {
            Placed::Here(message) => self.take(info, message).await,
            Placed::There(target, message) => self.deliver(target, message),
            Placed::Waiting => {}
        }
    }

    /// Where a message for `addressee` goes, as this worker sees membership now. Decided under
    /// the adoption lock, so a message that waits is released by the next takeover or
    /// membership change.
    fn place_for(&self, addressee: WorkerId, message: Message) -> Placed {
        let mut adoption = self.adoption.lock().unwrap();
        let target = self.routes.resolve(addressee);
        let responsible = addressee == self.id || adoption.adopted.contains(&addressee);
        if target == self.id && responsible {
            return Placed::Here(message);
        }
        if target != self.id && target != addressee {
            return Placed::There(target, message);
        }
        // This worker is the successor and has not taken over yet, or the sender knows of a
        // death this worker has not heard of yet.
        adoption.waiting.push(message);
        Placed::Waiting
    }

    /// Handles a message this worker is responsible for, now that the run is known here.
    async fn take(self: &Arc<Self>, info: Arc<RunInfo>, message: Message) {
        let run = info.id;
        if self.run_state(run).is_none() && self.install(info).await.is_none() {
            return self.drop_late(message);
        }
        let held = self.run_state(run).is_some_and(|state| match &message {
            Message::End(End { branch, .. }) => {
                state.scopes.lock().unwrap().contains_key(&branch.scope)
            }
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

    /// Tries placing `message` again after a failed log read.
    fn retry(self: &Arc<Self>, message: Message, run: RunId, err: anyhow::Error) {
        tracing::error!(%run, "reading the run failed, trying again: {err:#}");
        let worker = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(RETRY).await;
            worker.place(message).await;
        });
    }
}

enum Placed {
    Here(Message),
    There(WorkerId, Message),
    Waiting,
}
