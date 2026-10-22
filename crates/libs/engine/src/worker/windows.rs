//! Windowed scopes (design §4.5): `window(items, n)` closes the items' scope and continues once
//! per `n` items, each window a branch of a scope of its own. The owner of the items' scope opens
//! both: as the items' branches end, in branch order, every `n` items go out as a window branch,
//! and the last, partial window when every item has ended; then the windows' scope learns how
//! many it has. Each window is logged with the items' scope's progress, so a successor neither
//! loses nor repeats one.

use std::sync::Arc;

use namu_core::OpId;
use namu_core::value::Value;

use super::{Inner, Run};
use crate::ids::{ScopeId, child_lineage};
use crate::kernel::Start;
use crate::log::Record;
use crate::message::{Branch, BranchId, Resume};
use crate::scope::{Continuation, Feed, Fold, Scope, Then};
use crate::tree::Node;

/// The scopes a fan-out opens when a window closes its items' scope.
pub(super) struct Windowed {
    /// The windows' scope.
    pub into: ScopeId,
    /// How the items' scope feeds it.
    pub feed: Feed,
    /// The record of the windows' scope, to append with the items' scope's.
    pub record: Record,
}

impl Inner {
    /// Opens the windows' scope, if the scope being opened with `continuation` is closed by a
    /// window: it continues as the opening branch would have, and folds with `fold` if the scope
    /// that closes it reduces.
    pub(super) fn open_windows(
        self: &Arc<Self>,
        run: &Arc<Run>,
        continuation: &Continuation,
        ctx: &Arc<Node>,
        fold: Option<Fold>,
    ) -> Option<Windowed> {
        let op = continuation.collect?;
        let window = run.spec.workflow().operations[op].window()?;
        let into = self.ids.next();
        let continuation = Continuation {
            collect: run.spec.collect_ops[op],
            ..continuation.clone()
        };
        let record = Record::Scope {
            run: run.id(),
            scope: into,
            continuation: Some(continuation.clone()),
            source: None,
            count: None,
            feeds: None,
        };
        let then = Then::Continue {
            continuation,
            ctx: ctx.clone(),
        };
        let scope = Scope::new(then, None);
        let scope = match fold {
            Some(fold) => scope.with_fold(fold),
            None => scope,
        };
        run.open_scope(into, scope);
        let feed = Feed {
            size: window.size,
            op,
            start: 0,
            windows: 0,
        };
        Some(Windowed { into, feed, record })
    }

    /// Sends out the windows of scope `id` that are ready, and closes the scope it feeds once it
    /// is over.
    pub(super) fn send_windows(self: &Arc<Self>, run: &Arc<Run>, id: ScopeId) {
        let (ready, into, continuation, ctx, over) = {
            let mut scopes = run.scopes.lock().unwrap();
            let Some(scope) = scopes.get_mut(&id) else {
                return;
            };
            let Then::Feed {
                into,
                continuation,
                ctx,
            } = &scope.then
            else {
                return;
            };
            let (into, continuation, ctx) = (*into, continuation.clone(), ctx.clone());
            let ready = scope.next_windows();
            let over = match scope.is_over() {
                true => scopes.remove(&id),
                false => None,
            };
            (ready, into, continuation, ctx, over)
        };
        for (feed, items) in ready {
            self.emit_window(run, id, into, &continuation, &ctx, feed, items);
        }
        if let Some(scope) = over {
            self.scope_over(run, id, scope);
        }
    }

    /// Window `feed.windows - 1` (its items `items`) as a branch of scope `into`, logged with
    /// `feed`, scope `id`'s progress once it has gone out.
    #[allow(clippy::too_many_arguments)]
    fn emit_window(
        self: &Arc<Self>,
        run: &Arc<Run>,
        id: ScopeId,
        into: ScopeId,
        continuation: &Continuation,
        ctx: &Arc<Node>,
        feed: Feed,
        items: Vec<Value>,
    ) {
        let index = feed.windows - 1;
        let op: OpId = feed.op;
        let output = run.spec.workflow().operations[op]
            .window()
            .expect("a feed's op is a window")
            .output;
        let live = run.spec.liveness.after_action(op);
        let node = self.grow(run, ctx, live, vec![(output, Value::list(items))]);
        let scopes: Arc<[_]> = continuation.scopes.iter().copied().chain([into]).collect();
        let branch = Branch {
            id: BranchId { scope: into, index },
            hop: 0,
            lineage: window_lineage(continuation, index),
            clock: 0,
            ctx: node.id(),
            resume: Resume::Kernel(Start::After { op }),
            scopes,
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
            Record::Fed {
                run: run.id(),
                scope: id,
                into,
                feed,
            },
        ]);
        let release = self.release_after(1, self.gone(run, &branch));
        let to = self.destination();
        let branch = Branch { hop: 1, ..branch };
        self.hand_off(run, to, branch, vec![node, ctx.clone()], Some(release));
    }
}

/// Window `index`'s lineage: a child of the opening branch at the fan-out that made the items,
/// numbered from the top so that no item's number is a window's.
fn window_lineage(continuation: &Continuation, index: u64) -> u64 {
    child_lineage(
        continuation.lineage,
        continuation.clock - 1,
        u64::MAX - index,
    )
}
