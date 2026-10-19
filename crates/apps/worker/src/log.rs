//! The durable log in Redis (design §7). Run Redis with persistence (`appendonly yes`) for the
//! log to survive Redis restarts too.
//!
//! Appends go through one writer task, so they are applied in order, and are pipelined in batches;
//! an append is durable once its batch has been applied. Per run, the log is a handful of keys
//! under `namu:log:{run}:`; a set of live runs lets a successor find a dead worker's estate.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use namu_engine::RunInfo;
use namu_engine::ids::{NodeId, RunId, WorkerId, owner};
use namu_engine::log::{Durable, EndRecord, Log, Record, ReportRecord, RunEstate, ScopeEstate};
use namu_engine::message::{Branch, BranchId};
use namu_engine::scope::{Continuation, Position, Source};
use namu_engine::tree::Node;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use tokio::sync::{mpsc, oneshot};

/// Runs whose log is live.
const RUNS: &str = "namu:log:runs";

fn key(run: RunId, part: &str) -> String {
    format!("namu:log:{run}:{part}")
}

fn branch_field(holder: WorkerId, id: BranchId, hop: u64) -> String {
    format!("{holder}:{}:{}:{hop}", id.scope, id.index)
}

pub struct RedisLog {
    redis: ConnectionManager,
    appends: mpsc::UnboundedSender<(Vec<Record>, oneshot::Sender<()>)>,
}

impl RedisLog {
    pub fn spawn(redis: ConnectionManager) -> Arc<Self> {
        let (appends, queue) = mpsc::unbounded_channel();
        tokio::spawn(write(redis.clone(), queue));
        Arc::new(Self { redis, appends })
    }
}

/// Applies appends in order, a pipelined batch at a time.
async fn write(
    mut redis: ConnectionManager,
    mut queue: mpsc::UnboundedReceiver<(Vec<Record>, oneshot::Sender<()>)>,
) {
    while let Some(first) = queue.recv().await {
        let mut batch = vec![first];
        while batch.len() < 256 {
            match queue.try_recv() {
                Ok(next) => batch.push(next),
                Err(_) => break,
            }
        }
        let mut pipe = redis::pipe();
        for (records, _) in &batch {
            for record in records {
                if let Err(err) = add(&mut pipe, record) {
                    tracing::error!("unencodable log record dropped: {err:#}");
                }
            }
        }
        // Retry until it lands: an append that never becomes durable keeps its sender's copy, so
        // giving up would stall recovery rather than lose anything.
        let mut backoff = std::time::Duration::from_millis(50);
        while let Err(err) = pipe.query_async::<()>(&mut redis).await {
            tracing::error!("log write failed, retrying: {err}");
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(std::time::Duration::from_secs(2));
        }
        for (_, durable) in batch {
            let _ = durable.send(());
        }
    }
}

fn add(pipe: &mut redis::Pipeline, record: &Record) -> anyhow::Result<()> {
    match record {
        Record::Run(info) => {
            pipe.set(key(info.id, "info"), serde_json::to_string(info)?)
                .ignore()
                .sadd(RUNS, info.id.to_string())
                .ignore();
        }
        Record::Nodes { run, nodes } => {
            for node in nodes {
                pipe.hset(key(*run, "nodes"), node.id(), serde_json::to_string(node)?)
                    .ignore();
            }
        }
        Record::Branch {
            run,
            holder,
            branch,
        } => {
            let field = branch_field(*holder, branch.id, branch.hop);
            pipe.hset(key(*run, "branches"), field, serde_json::to_string(branch)?)
                .ignore();
        }
        Record::BranchGone {
            run,
            holder,
            id,
            hop,
        } => {
            pipe.hdel(key(*run, "branches"), branch_field(*holder, *id, *hop))
                .ignore();
        }
        Record::Scope {
            run,
            scope,
            continuation,
            source,
            count,
        } => {
            let record = serde_json::to_string(&(continuation, source))?;
            pipe.hset(key(*run, "scopes"), scope, record).ignore();
            if let Some(count) = count {
                pipe.hset(key(*run, "counts"), scope, count).ignore();
            }
        }
        Record::Position {
            run,
            scope,
            position,
        } => {
            let position = serde_json::to_string(position)?;
            pipe.hset(key(*run, "positions"), scope, position).ignore();
        }
        Record::Closed { run, scope, count } => {
            pipe.hset(key(*run, "counts"), scope, count).ignore();
        }
        Record::ScopeGone { run, scope } => {
            pipe.hdel(key(*run, "scopes"), scope).ignore();
            pipe.hdel(key(*run, "counts"), scope).ignore();
        }
        Record::End { run, scope, end } => {
            pipe.rpush(key(*run, "ends"), serde_json::to_string(&(scope, end))?)
                .ignore();
        }
        Record::Report { run, report } => {
            pipe.rpush(key(*run, "reports"), serde_json::to_string(report)?)
                .ignore();
        }
        Record::RunGone(run) => {
            let parts = [
                "info",
                "nodes",
                "branches",
                "scopes",
                "counts",
                "positions",
                "ends",
                "reports",
            ];
            let keys: Vec<String> = parts.iter().map(|part| key(*run, part)).collect();
            pipe.del(keys).ignore().srem(RUNS, run.to_string()).ignore();
        }
    }
    Ok(())
}

#[async_trait]
impl Log for RedisLog {
    fn append(&self, records: Vec<Record>) -> Durable {
        let (durable, done) = oneshot::channel();
        let _ = self.appends.send((records, durable));
        done
    }

    async fn node(&self, run: RunId, id: NodeId) -> anyhow::Result<Option<Arc<Node>>> {
        let raw: Option<String> = self.redis.clone().hget(key(run, "nodes"), id).await?;
        raw.map(|raw| serde_json::from_str(&raw).context("bad node in the log"))
            .transpose()
    }

    async fn run(&self, run: RunId) -> anyhow::Result<Option<Arc<RunInfo>>> {
        let raw: Option<String> = self.redis.clone().get(key(run, "info")).await?;
        raw.map(|raw| serde_json::from_str(&raw).context("bad run in the log"))
            .transpose()
    }

    async fn estate(&self, worker: WorkerId) -> anyhow::Result<Vec<RunEstate>> {
        let mut redis = self.redis.clone();
        let runs: Vec<String> = redis.smembers(RUNS).await?;
        let mut estates = Vec::new();
        for run in runs {
            let run: RunId = run.parse()?;
            let Some(info) = self.run(run).await? else {
                continue;
            };
            let branches: HashMap<String, String> = redis.hgetall(key(run, "branches")).await?;
            let held = format!("{worker}:");
            let branches = branches
                .into_iter()
                .filter(|(field, _)| field.starts_with(&held))
                .map(|(_, raw)| serde_json::from_str::<Branch>(&raw))
                .collect::<Result<Vec<_>, _>>()?;

            let scopes: HashMap<u64, String> = redis.hgetall(key(run, "scopes")).await?;
            let mut owned: HashMap<u64, (Option<Continuation>, Option<Source>)> = scopes
                .into_iter()
                .filter(|(scope, _)| owner(*scope) == worker)
                .map(|(scope, raw)| Ok((scope, serde_json::from_str(&raw)?)))
                .collect::<anyhow::Result<_>>()?;
            if owned.values().any(|(_, source)| source.is_some()) {
                let positions: HashMap<u64, String> = redis.hgetall(key(run, "positions")).await?;
                for (scope, raw) in positions {
                    if let Some((_, Some(source))) = owned.get_mut(&scope) {
                        source.position = serde_json::from_str::<Position>(&raw)?;
                    }
                }
            }
            let mut counts: HashMap<u64, u64> = HashMap::new();
            if !owned.is_empty() {
                counts = redis.hgetall(key(run, "counts")).await?;
            }
            let mut ends: HashMap<u64, Vec<EndRecord>> = HashMap::new();
            if !owned.is_empty() {
                let raw: Vec<String> = redis.lrange(key(run, "ends"), 0, -1).await?;
                for raw in raw {
                    let (scope, end): (u64, EndRecord) = serde_json::from_str(&raw)?;
                    if owned.contains_key(&scope) {
                        ends.entry(scope).or_default().push(end);
                    }
                }
            }
            let scopes: Vec<ScopeEstate> = owned
                .into_iter()
                .map(|(scope, (continuation, source))| ScopeEstate {
                    scope,
                    continuation,
                    source,
                    count: counts.get(&scope).copied(),
                    ends: ends.remove(&scope).unwrap_or_default(),
                })
                .collect();

            let reports = match info.owner() == worker {
                true => {
                    let raw: Vec<String> = redis.lrange(key(run, "reports"), 0, -1).await?;
                    raw.iter()
                        .map(|raw| serde_json::from_str::<ReportRecord>(raw))
                        .collect::<Result<Vec<_>, _>>()?
                }
                false => Vec::new(),
            };
            if branches.is_empty() && scopes.is_empty() && reports.is_empty() {
                continue;
            }
            estates.push(RunEstate {
                info: Some(info),
                reports,
                scopes,
                branches,
            });
        }
        Ok(estates)
    }
}
