//! Stream sources (design §6.5).
//!
//! A stream task whose output only feeds the fan-out after it runs as the source of that fan-out's
//! scope: each item it yields becomes a branch at once, and when it ends the scope knows its
//! count. The source runs on the worker that owns the scope, so its credits are local: it yields
//! only while fewer than `window` of the scope's branches are open, and every branch that ends
//! frees one. Its position is logged as it goes, so a successor can resume it inside the same
//! scope.

use std::sync::Arc;

use anyhow::Context;
use namu_core::OpId;
use namu_core::value::Value;
use tokio::sync::Semaphore;

use super::drive::{Driving, run_task};
use super::view::View;
use super::{Inner, Run};
use crate::ids::{ScopeId, child_lineage};
use crate::kernel::{self, Start, ValueStore};
use crate::log::Record;
use crate::message::{Branch, BranchId, Report, Resume};
use crate::scope::{Continuation, Fold, Position, Scope, Source, Then};
use crate::tasks::{Args, CallKey, Kind, Stream, StreamTask, Task, TaskStream};
use crate::tree::Node;

impl Inner {
    /// Runs the stream task called at `call` as the source of a new scope that feeds `fan_out`.
    /// The branch that called it waits in the scope.
    pub(super) fn start_stream(
        self: &Arc<Self>,
        run: &Arc<Run>,
        at: &mut Driving,
        call: OpId,
        fan_out: OpId,
        stream: Stream,
        fold: Option<Fold>,
    ) {
        let key = at.key(run, call);
        let branch = &at.branch;
        let scope = self.ids.next();
        let continuation = Continuation {
            branch: branch.id,
            hop: branch.hop,
            lineage: branch.lineage,
            clock: branch.clock,
            ctx: branch.ctx,
            scopes: branch.scopes.clone(),
            collect: run.spec.collect_ops[fan_out],
        };
        let source = Source {
            call,
            key,
            fan_out,
            position: Position::default(),
        };
        let release = self.release_after(1, self.gone(run, branch));
        let trail = self.trail(at);
        let durable = self.log.append(vec![
            Record::Nodes {
                run: run.id(),
                nodes: trail.clone(),
            },
            Record::Scope {
                run: run.id(),
                scope,
                continuation: Some(continuation.clone()),
                source: Some(source.clone()),
                count: None,
            },
        ]);
        self.release_when(durable, release);
        let then = Then::Continue {
            continuation,
            ctx: at.ctx.clone(),
        };
        let opened = Scope::new(then, None);
        let opened = match fold {
            Some(fold) => opened.with_fold(fold),
            None => opened,
        };
        self.feed_scope(run, scope, opened, source, stream, trail);
    }

    /// Gives `scope` (which may have ended branches already, after recovery) the credits it has
    /// left, and runs its source from its position. Every item is shipped with `trail`: the
    /// scope's context, and the nodes it hangs from that the receiver is unlikely to hold.
    pub(super) fn feed_scope(
        self: &Arc<Self>,
        run: &Arc<Run>,
        id: ScopeId,
        scope: Scope,
        source: Source,
        stream: Stream,
        trail: Vec<Arc<Node>>,
    ) {
        let Then::Continue { continuation, .. } = &scope.then else {
            unreachable!("a stream feeds a fan-out scope")
        };
        let in_flight = source.position.yielded - scope.ended();
        let credits = stream.window.saturating_sub(in_flight as usize);
        let credits = Arc::new(Semaphore::new(credits));
        let inner: Arc<[ScopeId]> = continuation.scopes.iter().copied().chain([id]).collect();
        run.open_scope(id, scope.with_credits(credits.clone()));

        let (worker, run) = (self.clone(), run.clone());
        let busy = self.busy();
        tokio::spawn(async move {
            let _busy = busy;
            let mut position = source.position.clone();
            let feeding = Feeding {
                scope: id,
                trail,
                inner,
                source: &source,
                stream: stream.task,
                credits: &credits,
            };
            // The source ends, and with it the scope's count. A failure takes the next index, as
            // a branch that failed.
            let count = match worker.feed(&run, &feeding, &mut position).await {
                Ok(Fed::Ended) => position.yielded,
                // The scope stays as it is, its source at its logged position, for the successor
                // that takes over from this worker.
                Ok(Fed::Drained) => return,
                Err(failure) => {
                    let branch = BranchId {
                        scope: id,
                        index: position.yielded,
                    };
                    worker.send_report(&run, branch, Report::Failure(failure), None);
                    position.yielded + 1
                }
            };
            worker.close(&run, id, count);
        });
    }

    /// Pulls items from the stream into the scope until it ends, or the worker drains. An error
    /// ends it early.
    async fn feed(
        self: &Arc<Self>,
        run: &Arc<Run>,
        feeding: &Feeding<'_>,
        position: &mut Position,
    ) -> Result<Fed, String> {
        let task = run
            .spec
            .task(feeding.source.call)
            .ok_or("a stream source is a call")?;
        let mut stream = self
            .open(run, feeding, position)
            .await
            .map_err(|err| format!("{}: {err:#}", task.id))??;
        let mut draining = self.draining.subscribe();
        loop {
            let credit = tokio::select! {
                credit = feeding.credits.acquire() => credit.map_err(|err| err.to_string())?,
                _ = draining.wait_for(|draining| *draining) => return Ok(Fed::Drained),
            };
            credit.forget();
            let item = {
                let _slot = self
                    .task_slots
                    .acquire()
                    .await
                    .map_err(|err| err.to_string())?;
                run_task(|| stream.next())
            };
            let item = match item {
                Ok(Some(Ok(item))) => item,
                Ok(None) => return Ok(Fed::Ended),
                Ok(Some(Err(message))) => return Err(format!("{}: {message}", task.id)),
                Err(err) => return Err(format!("{}: {err:#}", task.id)),
            };
            let index = position.yielded;
            position.yielded += 1;
            if let Some(cursor) = item.cursor {
                position.cursor = Some((cursor, position.yielded));
            }
            self.emit(run, feeding, index, item.value, position);
        }
    }

    /// Opens the stream at its position: at its last cursor, then skipped forward to the items
    /// not yet yielded.
    async fn open(
        self: &Arc<Self>,
        run: &Arc<Run>,
        feeding: &Feeding<'_>,
        position: &Position,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>> {
        let call = kernel::operation(run.spec.workflow(), feeding.source.call)?
            .call()
            .context("not a call")?;
        let view = View { worker: self, run };
        let mut args = Vec::with_capacity(call.inputs.len());
        for &input in &call.inputs {
            args.push(view.get(feeding.ctx(), input).await?);
        }
        let (cursor, at_cursor) = match &position.cursor {
            Some((cursor, at)) => (Some(cursor.clone()), *at),
            None => (None, 0),
        };
        let task = feeding.stream.clone();
        let key = feeding.source.key;
        let mut stream = match run_task(|| task.open(args, cursor, key))? {
            Ok(stream) => stream,
            Err(message) => return Ok(Err(message)),
        };
        for _ in at_cursor..position.yielded {
            match run_task(|| stream.next())? {
                Some(Ok(_)) => {}
                Some(Err(message)) => return Ok(Err(message)),
                None => break,
            }
        }
        Ok(Ok(stream))
    }

    /// Makes item `index` a branch of the scope, and hands it off. The item is logged as a
    /// branch held here in the same append that moves the source to `position`, and that record
    /// goes once the handoff is acknowledged: if this worker dies first, its successor either
    /// sends the item again or yields it again under the same index.
    fn emit(
        self: &Arc<Self>,
        run: &Arc<Run>,
        feeding: &Feeding<'_>,
        index: u64,
        value: Value,
        position: &Position,
    ) {
        let output = run.spec.workflow().operations[feeding.source.fan_out]
            .fan_out()
            .expect("a stream feeds a fan-out")
            .output;
        let live = run.spec.liveness.after_action(feeding.source.fan_out);
        let node = self.grow(run, feeding.ctx(), live, vec![(output, value)]);
        let branch = Branch {
            id: BranchId {
                scope: feeding.scope,
                index,
            },
            hop: 0,
            lineage: child_lineage(feeding.source.key.lineage, feeding.source.key.clock, index),
            clock: 0,
            ctx: node.id(),
            resume: Resume::Kernel(Start::After {
                op: feeding.source.fan_out,
            }),
            scopes: feeding.inner.clone(),
        };
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
            Record::Position {
                run: run.id(),
                scope: feeding.scope,
                position: position.clone(),
            },
        ]);
        let release = self.release_after(1, self.gone(run, &branch));
        let to = self.destination();
        let nodes = [node].into_iter().chain(feeding.trail.iter().cloned());
        let branch = Branch { hop: 1, ..branch };
        self.hand_off(run, to, branch, nodes.collect(), Some(release));
    }

    /// Runs a stream task whose output is not just fanned out, reading it to its end into a list.
    pub(super) async fn read_stream(
        &self,
        stream: &Stream,
        args: Args,
        key: CallKey,
    ) -> anyhow::Result<Result<Value, String>> {
        let _slot = self.task_slots.acquire().await?;
        let task = stream.task.clone();
        run_task(|| {
            let mut stream = match task.open(args, None, key)? {
                Ok(stream) => stream,
                Err(message) => return Ok(Err(message)),
            };
            let mut items = Vec::new();
            while let Some(item) = stream.next()? {
                match item {
                    Ok(item) => items.push(item.value),
                    Err(message) => return Ok(Err(message)),
                }
            }
            Ok(Ok(Value::list(items)))
        })
    }

    /// A branch of a stream's scope ended: the source may yield one more item.
    pub(super) fn free_credit(scope: &Scope) {
        if let Some(credits) = &scope.credits {
            credits.add_permits(1);
        }
    }

    /// Takes over a stream scope from a dead worker: its source resumes where it was, if this
    /// worker can run the stream task. If not, the branch that started the stream starts it again
    /// (from the beginning, in a new scope) on a worker that can, and this scope is abandoned.
    pub(super) fn adopt_stream(
        self: &Arc<Self>,
        run: &Arc<Run>,
        id: ScopeId,
        scope: Scope,
        source: Source,
    ) {
        let task = run
            .spec
            .task(source.call)
            .expect("a stream source is a call");
        if let Some(Task {
            kind: Kind::Stream(stream),
            ..
        }) = self.tasks.task(task)
        {
            let yielded = source.position.yielded;
            tracing::info!(task = task.id, yielded, "resuming a stream");
            let Then::Continue { ctx, .. } = &scope.then else {
                unreachable!("a stream feeds a fan-out scope")
            };
            let trail = vec![ctx.clone()];
            return self.feed_scope(run, id, scope, source, stream, trail);
        }
        tracing::info!(task = task.id, "restarting a stream where it is served");
        let Then::Continue { continuation, ctx } = scope.then else {
            unreachable!("a stream feeds a fan-out scope")
        };
        let branch = Branch {
            id: continuation.branch,
            hop: continuation.hop + 1,
            // The stream call is run again, under the key it had.
            lineage: continuation.lineage,
            clock: source.key.clock,
            ctx: continuation.ctx,
            resume: Resume::Action { op: source.call },
            scopes: continuation.scopes,
        };
        self.log.record(vec![
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
        self.spawn_drive(run.clone(), branch, ctx);
    }
}

/// How feeding a scope stopped.
enum Fed {
    /// The stream ended.
    Ended,
    /// The worker drains; the source resumes on its successor.
    Drained,
}

/// A source and the scope it feeds.
struct Feeding<'a> {
    scope: ScopeId,
    /// The context the stream task was called in, which items are children of, first; then the
    /// nodes it hangs from that items are shipped with.
    trail: Vec<Arc<Node>>,
    /// The scopes items are in, innermost (this one) last.
    inner: Arc<[ScopeId]>,
    source: &'a Source,
    stream: Arc<dyn StreamTask>,
    credits: &'a Semaphore,
}

impl Feeding<'_> {
    fn ctx(&self) -> &Arc<Node> {
        &self.trail[0]
    }
}
