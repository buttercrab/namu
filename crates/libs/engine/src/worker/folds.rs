//! Reducing scopes (design §4.5): the scope's owner folds its items with the reduce task, in
//! branch order, one step at a time, as the branches end. Each step is logged with the
//! accumulator it made, so a successor goes on from the last one; a step done again has the key
//! it had, its item's.

use std::sync::Arc;

use namu_core::value::Value;

use super::{Inner, Run};
use crate::ids::{ScopeId, child_lineage};
use crate::log::Record;
use crate::message::{BranchId, Report};
use crate::scope::{Continuation, Then};
use crate::tasks::CallKey;

impl Inner {
    /// Starts scope `id`'s next fold step, if an item is ready and no step is running. Each step
    /// starts the next one when it is done.
    pub(super) fn fold(self: &Arc<Self>, run: &Arc<Run>, id: ScopeId) {
        let step = {
            let mut scopes = run.scopes.lock().unwrap();
            let Some(scope) = scopes.get_mut(&id) else {
                return;
            };
            match scope.next_fold() {
                Some((index, item)) => {
                    let fold = scope.fold.as_ref().expect("the scope folds");
                    let Then::Continue { continuation, .. } = &scope.then else {
                        unreachable!("the root scope does not fold")
                    };
                    let key = fold_key(run, continuation, fold.op, index);
                    Some((
                        fold.task.clone(),
                        fold.accumulator.clone(),
                        item,
                        index,
                        key,
                    ))
                }
                // Passing over branches that brought no item may have finished the fold.
                None if scope.is_over() => {
                    let scope = scopes.remove(&id).expect("held above");
                    drop(scopes);
                    return self.scope_over(run, id, scope);
                }
                None => None,
            }
        };
        let Some((task_ref, accumulator, item, index, key)) = step else {
            return;
        };
        // Counted as work in flight: a draining worker waits for it.
        let busy = self.busy();
        let (worker, run) = (self.clone(), run.clone());
        tokio::spawn(async move {
            let output = match worker.tasks.task(&task_ref) {
                Some(task) => {
                    let args = vec![accumulator.clone(), item];
                    let output = worker.invoke(&task_ref, &task, args, key).await;
                    run.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    output
                }
                None => Ok(Err(
                    "not served by the worker that owns its scope".to_string()
                )),
            };
            let accumulator = match output {
                Ok(Ok(next)) => next,
                // A step that fails drops its item, as a failed branch is dropped from a collect.
                failed => {
                    let message = match failed {
                        Ok(Err(message)) => message,
                        Err(err) => format!("{err:#}"),
                        Ok(Ok(_)) => unreachable!(),
                    };
                    let failure = Report::Failure(format!("{}: {message}", task_ref.id));
                    worker.send_report(&run, BranchId { scope: id, index }, failure, None);
                    accumulator
                }
            };
            worker.folded(&run, id, accumulator);
            drop(busy);
        });
    }

    /// The running step of scope `id` made `accumulator`.
    fn folded(self: &Arc<Self>, run: &Arc<Run>, id: ScopeId, accumulator: Value) {
        let over = {
            let mut scopes = run.scopes.lock().unwrap();
            let Some(scope) = scopes.get_mut(&id) else {
                return;
            };
            scope.folded(accumulator.clone());
            let through = scope.fold.as_ref().expect("the scope folds").next;
            self.log.record(vec![Record::Folded {
                run: run.id(),
                scope: id,
                through,
                accumulator,
            }]);
            match scope.is_over() {
                true => scopes.remove(&id),
                false => None,
            }
        };
        match over {
            Some(scope) => self.scope_over(run, id, scope),
            None => self.fold(run, id),
        }
    }
}

/// The key of the fold step for item `index`: named after the item's branch (its lineage), at a
/// clock none of that branch's own calls reaches.
fn fold_key(run: &Run, continuation: &Continuation, op: usize, index: u64) -> CallKey {
    // The opening branch's clock ticked at the fan-out (or stream call) that made the items.
    let clock = continuation.clock - 1;
    CallKey {
        run: run.id(),
        lineage: child_lineage(continuation.lineage, clock, index),
        clock: u64::MAX,
        op,
    }
}
