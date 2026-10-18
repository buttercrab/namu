//! The distributed value tree.
//!
//! Nodes are immutable, including their parent. A worker keeps the nodes it has seen in memory;
//! nodes created by other workers are fetched from Redis once and cached. Nodes this worker
//! creates go into the local cache immediately and reach Redis through a write-behind
//! [`Writer`], so a local step never waits on the network. Before handing work to another
//! machine the worker flushes the writer, which guarantees the receiver can read every node it
//! may need.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use namu_engine::kernel::ValueStore;
use namu_proto::{NodeRecord, keys};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde_json::Value as JsonValue;
use tokio::sync::{mpsc, oneshot};
use tracing::error;
use uuid::Uuid;

pub type Node = NodeRecord;

enum WriteOp {
    Set { key: String, value: String },
    HIncr { key: String, field: String, by: i64 },
    Barrier(oneshot::Sender<()>),
}

/// Write-behind channel to Redis. Operations are applied in order, in pipelined batches.
#[derive(Clone)]
pub struct Writer {
    tx: mpsc::UnboundedSender<WriteOp>,
}

impl Writer {
    pub fn spawn(mut redis: ConnectionManager) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<WriteOp>();
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                while batch.len() < 512 {
                    match rx.try_recv() {
                        Ok(op) => batch.push(op),
                        Err(_) => break,
                    }
                }
                let mut pipe = redis::pipe();
                let mut writes = 0;
                let mut barriers = Vec::new();
                for op in batch {
                    match op {
                        WriteOp::Set { key, value } => {
                            pipe.set(key, value).ignore();
                            writes += 1;
                        }
                        WriteOp::HIncr { key, field, by } => {
                            pipe.hincr(key, field, by).ignore();
                            writes += 1;
                        }
                        WriteOp::Barrier(ack) => barriers.push(ack),
                    }
                }
                // An empty pipeline would wait forever for zero replies.
                if writes > 0
                    && let Err(err) = pipe.query_async::<()>(&mut redis).await
                {
                    error!("write-behind batch failed: {err}");
                }
                for ack in barriers {
                    let _ = ack.send(());
                }
            }
        });
        Self { tx }
    }

    pub fn set(&self, key: String, value: String) {
        let _ = self.tx.send(WriteOp::Set { key, value });
    }

    pub fn hincr(&self, key: String, field: String, by: i64) {
        let _ = self.tx.send(WriteOp::HIncr { key, field, by });
    }

    /// Waits until every write issued before this call is applied in Redis.
    pub async fn flush(&self) {
        let (ack, done) = oneshot::channel();
        if self.tx.send(WriteOp::Barrier(ack)).is_ok() {
            let _ = done.await;
        }
    }
}

/// Mints globally unique context ids for this worker (see `namu_proto::context_id`).
pub struct IdMinter {
    worker_index: u64,
    counter: AtomicU64,
}

impl IdMinter {
    pub fn new(worker_index: u64) -> Self {
        Self {
            worker_index,
            counter: AtomicU64::new(1),
        }
    }

    pub fn next(&self) -> u64 {
        namu_proto::context_id(
            self.worker_index,
            self.counter.fetch_add(1, Ordering::Relaxed),
        )
    }
}

/// One run's view of the tree on this worker.
pub struct RunTree {
    run_id: Uuid,
    worker_id: String,
    nodes: Mutex<HashMap<u64, Arc<Node>>>,
    redis: ConnectionManager,
    writer: Writer,
    ids: Arc<IdMinter>,
}

impl RunTree {
    pub fn new(
        run_id: Uuid,
        worker_id: String,
        redis: ConnectionManager,
        writer: Writer,
        ids: Arc<IdMinter>,
    ) -> Self {
        Self {
            run_id,
            worker_id,
            nodes: Mutex::new(HashMap::new()),
            redis,
            writer,
            ids,
        }
    }

    pub async fn node(&self, ctx: u64) -> anyhow::Result<Arc<Node>> {
        if let Some(node) = self.nodes.lock().unwrap().get(&ctx) {
            return Ok(node.clone());
        }
        // Produced elsewhere: read it once from Redis. Nodes never change, so caching is safe.
        tracing::debug!(ctx, "node: fetch");
        let mut redis = self.redis.clone();
        let raw: Option<String> = redis.get(keys::node(self.run_id, ctx)).await?;
        tracing::debug!(ctx, found = raw.is_some(), "node: fetched");
        let raw = raw.ok_or_else(|| anyhow::anyhow!("missing value-tree node {ctx}"))?;
        let node: Arc<Node> = Arc::new(serde_json::from_str(&raw)?);
        self.writer.hincr(
            keys::run_remote_fetches(self.run_id),
            self.worker_id.clone(),
            1,
        );
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .entry(ctx)
            .or_insert(node)
            .clone())
    }

    /// Creates an immutable node and returns its id.
    pub fn create(&self, node: Node) -> anyhow::Result<u64> {
        let id = self.ids.next();
        let json = serde_json::to_string(&node)?;
        self.nodes.lock().unwrap().insert(id, Arc::new(node));
        self.writer.set(keys::node(self.run_id, id), json);
        Ok(id)
    }

    /// Creates a child of `parent` that inherits its fan-out scopes.
    pub async fn create_child(
        &self,
        parent: u64,
        values: Vec<(usize, JsonValue)>,
    ) -> anyhow::Result<u64> {
        let scopes = self.node(parent).await?.scopes.clone();
        self.create(Node {
            parent: Some(parent),
            scopes,
            values,
        })
    }

    pub async fn get_value(&self, ctx: u64, val_id: usize) -> anyhow::Result<JsonValue> {
        let mut cursor = Some(ctx);
        while let Some(id) = cursor {
            let node = self.node(id).await?;
            if let Some((_, value)) = node.values.iter().find(|(v, _)| *v == val_id) {
                return Ok(value.clone());
            }
            cursor = node.parent;
        }
        Err(anyhow::anyhow!("missing value {val_id} in ctx {ctx}"))
    }
}

/// Adapts a [`RunTree`] to the kernel's store interface. Every write creates a new node.
pub struct TreeStore<'a>(pub &'a RunTree);

#[async_trait]
impl ValueStore for TreeStore<'_> {
    type Value = JsonValue;

    async fn get_value(&self, ctx_id: usize, val_id: usize) -> anyhow::Result<JsonValue> {
        self.0.get_value(ctx_id as u64, val_id).await
    }

    async fn get_values(&self, ctx_id: usize, val_ids: &[usize]) -> anyhow::Result<Vec<JsonValue>> {
        let mut out = Vec::with_capacity(val_ids.len());
        for &val_id in val_ids {
            out.push(self.0.get_value(ctx_id as u64, val_id).await?);
        }
        Ok(out)
    }

    async fn set_value(
        &self,
        ctx_id: usize,
        val_id: usize,
        value: JsonValue,
    ) -> anyhow::Result<usize> {
        self.set_values(ctx_id, vec![(val_id, value)]).await
    }

    async fn set_values(
        &self,
        ctx_id: usize,
        values: Vec<(usize, JsonValue)>,
    ) -> anyhow::Result<usize> {
        Ok(self.0.create_child(ctx_id as u64, values).await? as usize)
    }
}
