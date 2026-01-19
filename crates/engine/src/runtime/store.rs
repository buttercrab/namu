use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use namu_core::{ContextId, ValueId};

use crate::runtime::graph::ContextGraph;

#[async_trait]
pub trait ValueStore: Send + Sync {
    type Value: Clone + Send + Sync + 'static;

    async fn get_value(&self, ctx_id: ContextId, val_id: ValueId) -> anyhow::Result<Self::Value>;
    async fn get_values(
        &self,
        ctx_id: ContextId,
        val_ids: &[ValueId],
    ) -> anyhow::Result<Vec<Self::Value>>;
    async fn set_value(
        &self,
        ctx_id: ContextId,
        val_id: ValueId,
        value: Self::Value,
    ) -> anyhow::Result<ContextId>;
}

#[derive(Debug, Clone)]
struct ContextNode<V> {
    parent: Option<ContextId>,
    values: HashMap<ValueId, V>,
}

#[derive(Debug, Default)]
pub struct InMemoryStore<V>
where
    V: Clone + Send + Sync + 'static,
{
    next_id: AtomicUsize,
    nodes: RwLock<HashMap<ContextId, ContextNode<V>>>,
}

impl<V> InMemoryStore<V>
where
    V: Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            next_id: AtomicUsize::new(0),
            nodes: RwLock::new(HashMap::new()),
        }
    }

    pub fn create_root(&self) -> ContextId {
        self.create_child_inner(None)
    }

    pub fn create_child(&self, parent: ContextId) -> ContextId {
        self.create_child_inner(Some(parent))
    }

    fn create_child_inner(&self, parent: Option<ContextId>) -> ContextId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut nodes = self.nodes.write().expect("store lock poisoned");
        nodes.insert(
            id,
            ContextNode {
                parent,
                values: HashMap::new(),
            },
        );
        id
    }

    pub fn finish_context(&self, _ctx_id: ContextId) {
        // No-op for now; child contexts may still depend on parent values.
    }
}

#[async_trait]
impl<V> ContextGraph for InMemoryStore<V>
where
    V: Clone + Send + Sync + 'static,
{
    async fn parent(&self, ctx_id: ContextId) -> anyhow::Result<Option<ContextId>> {
        let nodes = self.nodes.read().expect("store lock poisoned");
        let node = nodes
            .get(&ctx_id)
            .ok_or_else(|| anyhow::anyhow!("missing context {ctx_id}"))?;
        Ok(node.parent)
    }
}

#[async_trait]
impl<V> ValueStore for InMemoryStore<V>
where
    V: Clone + Send + Sync + 'static,
{
    type Value = V;

    async fn get_value(&self, ctx_id: ContextId, val_id: ValueId) -> anyhow::Result<Self::Value> {
        let nodes = self.nodes.read().expect("store lock poisoned");
        let mut cursor = Some(ctx_id);
        while let Some(id) = cursor {
            let node = nodes
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("missing context {id}"))?;
            if let Some(value) = node.values.get(&val_id) {
                return Ok(value.clone());
            }
            cursor = node.parent;
        }
        Err(anyhow::anyhow!("missing value {val_id} in ctx {ctx_id}"))
    }

    async fn get_values(
        &self,
        ctx_id: ContextId,
        val_ids: &[ValueId],
    ) -> anyhow::Result<Vec<Self::Value>> {
        let mut out = Vec::with_capacity(val_ids.len());
        for &val_id in val_ids {
            out.push(self.get_value(ctx_id, val_id).await?);
        }
        Ok(out)
    }

    async fn set_value(
        &self,
        ctx_id: ContextId,
        val_id: ValueId,
        value: Self::Value,
    ) -> anyhow::Result<ContextId> {
        let mut nodes = self.nodes.write().expect("store lock poisoned");
        let node = nodes
            .get_mut(&ctx_id)
            .ok_or_else(|| anyhow::anyhow!("missing context {ctx_id}"))?;
        node.values.insert(val_id, value);
        Ok(ctx_id)
    }
}
