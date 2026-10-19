//! The durable log in Redis (design §7). Run Redis with persistence (`appendonly yes`) for the
//! log to survive Redis restarts too. Records are MessagePack.
//!
//! Appends go through one writer task, so they are applied in order, a batch at a time in one
//! transaction; an append is durable once its batch has been applied. Per run, the log is a
//! handful of keys under `namu:log:{run}:`; a set of live runs lets a successor find a dead
//! worker's estate.
//!
//! The log holds a run's live frontier, not its history (design §7.5). A scope's ends, count and
//! position go with the scope, and nodes nothing live reaches are swept: after a batch that removed
//! records, once a run's nodes have doubled since its last sweep. A sweep reads nodes and records
//! in one transaction, and every append is a transaction too, so it never sees a node without the
//! record that refers to it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use async_trait::async_trait;
use namu_engine::RunInfo;
use namu_engine::ids::{NodeId, RunId, WorkerId, owner};
use namu_engine::log::{
    Durable, EndRecord, Log, Record, ReportRecord, RunEstate, SWEEP_FLOOR, ScopeEstate, dead_nodes,
};
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

/// The ends of one scope.
fn ends_key(run: RunId, scope: u64) -> String {
    key(run, &format!("ends:{scope}"))
}

fn encode(record: &impl serde::Serialize) -> anyhow::Result<Vec<u8>> {
    Ok(rmp_serde::to_vec_named(record)?)
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
    let sweeps = Sweeps::default();
    while let Some(first) = queue.recv().await {
        let mut batch = vec![first];
        while batch.len() < 256 {
            match queue.try_recv() {
                Ok(next) => batch.push(next),
                Err(_) => break,
            }
        }
        let mut pipe = redis::pipe();
        pipe.atomic();
        // Runs that lost records in this batch: their dead nodes may be worth a sweep.
        let mut shrunk = HashSet::new();
        for (records, _) in &batch {
            for record in records {
                if let Err(err) = add(&mut pipe, record) {
                    tracing::error!("unencodable log record dropped: {err:#}");
                }
                if let Record::BranchGone { run, .. } | Record::ScopeGone { run, .. } = record {
                    shrunk.insert(*run);
                }
            }
        }
        // Retry until it lands: an append that never becomes durable keeps its sender's copy, so
        // giving up would stall recovery rather than lose anything. A batch with nothing to write
        // (an empty append marks what came before as durable) is durable already.
        let mut backoff = std::time::Duration::from_millis(50);
        let empty = pipe.cmd_iter().next().is_none();
        while !empty && let Err(err) = pipe.query_async::<()>(&mut redis).await {
            tracing::error!("log write failed, retrying: {err}");
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(std::time::Duration::from_secs(2));
        }
        for (_, durable) in batch {
            let _ = durable.send(());
        }
        for run in shrunk {
            sweeps.start(&redis, run);
        }
    }
}

/// The runs being swept by this worker, so each is swept by at most one task at a time.
#[derive(Clone, Default)]
struct Sweeps(Arc<Mutex<HashSet<RunId>>>);

impl Sweeps {
    fn start(&self, redis: &ConnectionManager, run: RunId) {
        if !self.0.lock().unwrap().insert(run) {
            return;
        }
        let (sweeps, mut redis) = (self.clone(), redis.clone());
        tokio::spawn(async move {
            if let Err(err) = sweep(&mut redis, run).await {
                tracing::error!(%run, "sweeping the log failed: {err:#}");
            }
            sweeps.0.lock().unwrap().remove(&run);
        });
    }
}

/// Drops the run's nodes that nothing live reaches, if its nodes have doubled since the last
/// sweep.
async fn sweep(redis: &mut ConnectionManager, run: RunId) -> anyhow::Result<()> {
    let (held, swept): (usize, Option<usize>) = redis::pipe()
        .hlen(key(run, "parents"))
        .get(key(run, "swept"))
        .query_async(redis)
        .await?;
    if held < SWEEP_FLOOR.max(2 * swept.unwrap_or(0)) {
        return Ok(());
    }
    // One snapshot of nodes and the records that keep them live.
    type Snapshot = (
        HashMap<NodeId, Vec<u8>>,
        HashMap<String, Vec<u8>>,
        HashMap<u64, Vec<u8>>,
    );
    let (parents, branches, scopes): Snapshot = redis::pipe()
        .atomic()
        .hgetall(key(run, "parents"))
        .hgetall(key(run, "branches"))
        .hgetall(key(run, "scopes"))
        .query_async(redis)
        .await?;
    let parents = parents
        .into_iter()
        .map(|(id, parent)| Ok((id, rmp_serde::from_slice(&parent)?)))
        .collect::<anyhow::Result<HashMap<NodeId, Option<NodeId>>>>()?;
    let mut live = Vec::new();
    for raw in branches.values() {
        live.push(rmp_serde::from_slice::<Branch>(raw)?.ctx);
    }
    for raw in scopes.values() {
        let (continuation, _): (Option<Continuation>, Option<Source>) = rmp_serde::from_slice(raw)?;
        live.extend(continuation.map(|c| c.ctx));
    }
    let dead = dead_nodes(&parents, live);
    let mut pipe = redis::pipe();
    pipe.atomic();
    if !dead.is_empty() {
        pipe.hdel(key(run, "nodes"), &dead)
            .ignore()
            .hdel(key(run, "parents"), &dead)
            .ignore();
    }
    pipe.set(key(run, "swept"), parents.len() - dead.len())
        .ignore();
    pipe.query_async::<()>(redis).await?;
    tracing::debug!(%run, dropped = dead.len(), "swept the log");
    Ok(())
}

fn add(pipe: &mut redis::Pipeline, record: &Record) -> anyhow::Result<()> {
    match record {
        Record::Run(info) => {
            pipe.set(key(info.id, "info"), encode(info)?)
                .ignore()
                .sadd(RUNS, info.id.to_string())
                .ignore();
        }
        Record::Nodes { run, nodes } => {
            for node in nodes {
                pipe.hset(key(*run, "nodes"), node.id(), encode(node)?)
                    .ignore()
                    .hset(key(*run, "parents"), node.id(), encode(&node.parent_id())?)
                    .ignore();
            }
        }
        Record::Branch {
            run,
            holder,
            branch,
        } => {
            let field = branch_field(*holder, branch.id, branch.hop);
            pipe.hset(key(*run, "branches"), field, encode(branch)?)
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
            let record = encode(&(continuation, source))?;
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
            let position = encode(position)?;
            pipe.hset(key(*run, "positions"), scope, position).ignore();
        }
        Record::Closed { run, scope, count } => {
            pipe.hset(key(*run, "counts"), scope, count).ignore();
        }
        Record::ScopeGone { run, scope } => {
            pipe.hdel(key(*run, "scopes"), scope)
                .ignore()
                .hdel(key(*run, "counts"), scope)
                .ignore()
                .hdel(key(*run, "positions"), scope)
                .ignore()
                .del(ends_key(*run, *scope))
                .ignore();
        }
        Record::End { run, scope, end } => {
            pipe.rpush(ends_key(*run, *scope), encode(end)?).ignore();
        }
        Record::Report { run, report } => {
            pipe.rpush(key(*run, "reports"), encode(report)?).ignore();
        }
        Record::RunGone { run, root_scope } => {
            // Every other scope is gone by now, and its ends with it.
            let parts = [
                "info",
                "nodes",
                "parents",
                "swept",
                "branches",
                "scopes",
                "counts",
                "positions",
                "reports",
            ];
            let mut keys: Vec<String> = parts.iter().map(|part| key(*run, part)).collect();
            keys.push(ends_key(*run, *root_scope));
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
        let raw: Option<Vec<u8>> = self.redis.clone().hget(key(run, "nodes"), id).await?;
        raw.map(|raw| rmp_serde::from_slice(&raw).context("bad node in the log"))
            .transpose()
    }

    async fn run(&self, run: RunId) -> anyhow::Result<Option<Arc<RunInfo>>> {
        let raw: Option<Vec<u8>> = self.redis.clone().get(key(run, "info")).await?;
        raw.map(|raw| rmp_serde::from_slice(&raw).context("bad run in the log"))
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
            let branches: HashMap<String, Vec<u8>> = redis.hgetall(key(run, "branches")).await?;
            let held = format!("{worker}:");
            let branches = branches
                .into_iter()
                .filter(|(field, _)| field.starts_with(&held))
                .map(|(_, raw)| rmp_serde::from_slice::<Branch>(&raw))
                .collect::<Result<Vec<_>, _>>()?;

            let scopes: HashMap<u64, Vec<u8>> = redis.hgetall(key(run, "scopes")).await?;
            let mut owned: HashMap<u64, (Option<Continuation>, Option<Source>)> = scopes
                .into_iter()
                .filter(|(scope, _)| owner(*scope) == worker)
                .map(|(scope, raw)| Ok((scope, rmp_serde::from_slice(&raw)?)))
                .collect::<anyhow::Result<_>>()?;
            if owned.values().any(|(_, source)| source.is_some()) {
                let positions: HashMap<u64, Vec<u8>> = redis.hgetall(key(run, "positions")).await?;
                for (scope, raw) in positions {
                    if let Some((_, Some(source))) = owned.get_mut(&scope) {
                        source.position = rmp_serde::from_slice::<Position>(&raw)?;
                    }
                }
            }
            let mut counts: HashMap<u64, u64> = HashMap::new();
            if !owned.is_empty() {
                counts = redis.hgetall(key(run, "counts")).await?;
            }
            let mut ends: HashMap<u64, Vec<EndRecord>> = HashMap::new();
            for &scope in owned.keys() {
                let raw: Vec<Vec<u8>> = redis.lrange(ends_key(run, scope), 0, -1).await?;
                let records = raw
                    .iter()
                    .map(|raw| rmp_serde::from_slice(raw))
                    .collect::<Result<Vec<EndRecord>, _>>()?;
                ends.insert(scope, records);
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
                    let raw: Vec<Vec<u8>> = redis.lrange(key(run, "reports"), 0, -1).await?;
                    raw.iter()
                        .map(|raw| rmp_serde::from_slice::<ReportRecord>(raw))
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
