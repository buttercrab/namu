//! A run's value tree as the kernel sees it from this worker.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, bail};
use namu_core::ValueId;
use serde_json::Value;
use tokio::sync::oneshot;

use super::{Inner, Run};
use crate::ids::{NodeId, RunId, owner};
use crate::kernel::ValueStore;
use crate::message::Message;
use crate::tree::Node;

pub(super) struct View<'a> {
    pub worker: &'a Arc<Inner>,
    pub run: &'a Run,
}

impl ValueStore for View<'_> {
    async fn get(&self, ctx: NodeId, value: ValueId) -> anyhow::Result<Value> {
        let mut cursor = Some(ctx);
        while let Some(id) = cursor {
            let node = self.worker.node(self.run, id).await?;
            if let Some(found) = node.get(value) {
                return Ok(found.clone());
            }
            cursor = node.parent;
        }
        bail!("value {value} is not visible from context {ctx}")
    }

    async fn put(&self, ctx: NodeId, values: Vec<(ValueId, Value)>) -> anyhow::Result<NodeId> {
        let id = self.worker.ids.next();
        let node = Node {
            parent: Some(ctx),
            values,
        };
        self.run.nodes.insert(id, Arc::new(node));
        Ok(id)
    }
}

impl Inner {
    /// A node of `run`: held here, fetched from the worker that created it, or read from the log
    /// if that worker is dead. Concurrent requests for one node share one fetch.
    async fn node(self: &Arc<Self>, run: &Run, id: NodeId) -> anyhow::Result<Arc<Node>> {
        if let Some(node) = run.nodes.get(id) {
            return Ok(node);
        }
        let holder = owner(id);
        let node = if self.routes.resolve(holder) != holder {
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
            reply.await.context("node fetch abandoned")?
        };
        let node = node.with_context(|| format!("node {id} is gone"))?;
        run.nodes.insert(id, node.clone());
        Ok(node)
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
