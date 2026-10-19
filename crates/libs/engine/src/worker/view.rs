//! A run's value tree as the kernel sees it from this worker.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, bail};
use namu_core::ValueId;
use serde_json::Value;
use tokio::sync::oneshot;

use super::{Inner, Run};
use crate::ids::{NodeId, RunId, owner};
use crate::kernel::{Point, ValueStore};
use crate::message::Message;
use crate::tree::Node;

pub(super) struct View<'a> {
    pub worker: &'a Arc<Inner>,
    pub run: &'a Run,
}

impl ValueStore for View<'_> {
    type Ctx = Arc<Node>;

    async fn get(&self, ctx: &Arc<Node>, value: ValueId) -> anyhow::Result<Value> {
        let mut node = ctx.clone();
        loop {
            if let Some(found) = node.get(value) {
                return Ok(found.clone());
            }
            node = match self.worker.parent(self.run, &node).await? {
                Some(parent) => parent,
                None => bail!("value {value} is not visible from context {}", ctx.id()),
            };
        }
    }

    async fn put(
        &self,
        ctx: &Arc<Node>,
        values: Vec<(ValueId, Value)>,
        at: Point,
    ) -> anyhow::Result<Arc<Node>> {
        let live = match at {
            Point::Enter(op) => self.run.spec.liveness.after_enter(op),
            Point::Action(op) => self.run.spec.liveness.after_action(op),
        };
        Ok(self.worker.grow(self.run, ctx, live, values))
    }
}

impl Inner {
    /// Creates a node holding `values` on `ctx`'s chain, under the deepest node that still holds
    /// one of `live` (design §6.2); nodes below that hold only dead values and are left behind.
    pub(super) fn grow(
        &self,
        run: &Run,
        ctx: &Arc<Node>,
        live: &[ValueId],
        values: Vec<(ValueId, Value)>,
    ) -> Arc<Node> {
        // What the new node defines shadows the same values further up: a loop's phi defines
        // the same value every iteration, and the old copies are as dead as the rest.
        let live: Vec<ValueId> = live
            .iter()
            .copied()
            .filter(|value| !values.iter().any(|(id, _)| id == value))
            .collect();
        let node = Node::new(self.ids.next(), anchor(ctx, &live).clone(), values);
        run.nodes.hold(Arc::new(node))
    }
}

/// The deepest node of `ctx`'s chain that holds one of `live`: what a new node attaches under.
/// The walk stops at a node whose parent is not held here (or the root), keeping it.
fn anchor<'a>(ctx: &'a Arc<Node>, live: &[ValueId]) -> &'a Arc<Node> {
    let mut cursor = ctx;
    while !cursor.holds_any(live)
        && let Some(parent) = cursor.parent()
    {
        cursor = parent;
    }
    cursor
}

impl Inner {
    /// The parent of `node`, which `node` holds from now on. `None` for the root.
    async fn parent(self: &Arc<Self>, run: &Run, node: &Node) -> anyhow::Result<Option<Arc<Node>>> {
        if let Some(parent) = node.parent() {
            return Ok(Some(parent.clone()));
        }
        let Some(id) = node.parent_id() else {
            return Ok(None);
        };
        let parent = self.node(run, id).await?;
        node.link(parent.clone());
        Ok(Some(parent))
    }

    /// A node of `run`: held here, fetched from the worker that created it, or read from the log
    /// if that worker is dead. Concurrent requests for one node share one fetch.
    pub(super) async fn node(self: &Arc<Self>, run: &Run, id: NodeId) -> anyhow::Result<Arc<Node>> {
        if let Some(node) = run.nodes.get(id) {
            return Ok(node);
        }
        let holder = owner(id);
        // Nodes held by a dead worker, or no longer held by their creator, are in the log.
        let node = if self.routes.resolve(holder) != holder || holder == self.id {
            self.log.node(run.id(), id).await?
        } else {
            let (waiter, reply) = oneshot::channel();
            let first = {
                let mut fetches = self.fetches.lock().unwrap();
                let waiters = fetches.entry((run.id(), id)).or_default();
                waiters.push(waiter);
                waiters.len() == 1
            };
            if first {
                let reply_to = self.id;
                let fetch = Message::FetchNode {
                    run: run.id(),
                    id,
                    reply_to,
                };
                self.deliver(holder, fetch);
                run.fetches.fetch_add(1, Ordering::Relaxed);
            }
            match reply.await.context("node fetch abandoned")? {
                Some(node) => Some(node),
                // Its creator no longer holds it.
                None => self.log.node(run.id(), id).await?,
            }
        };
        let node = node.with_context(|| format!("node {id} is gone"))?;
        Ok(run.nodes.hold(node))
    }

    pub(super) fn on_node(&self, run: RunId, id: NodeId, node: Option<Arc<Node>>) {
        let waiters = self.fetches.lock().unwrap().remove(&(run, id));
        for waiter in waiters.into_iter().flatten() {
            let _ = waiter.send(node.clone());
        }
    }

    /// Fetches waiting on a worker that died are answered from the log instead.
    pub(super) fn refetch_from_log(self: &Arc<Self>) {
        let orphaned: Vec<(RunId, NodeId)> = self
            .fetches
            .lock()
            .unwrap()
            .keys()
            .filter(|(_, id)| self.routes.resolve(owner(*id)) != owner(*id))
            .copied()
            .collect();
        for (run, id) in orphaned {
            let worker = self.clone();
            tokio::spawn(async move {
                let node = worker.log.node(run, id).await.unwrap_or_else(|err| {
                    tracing::error!("reading node {id} from the log failed: {err:#}");
                    None
                });
                worker.on_node(run, id, node);
            });
        }
    }
}
