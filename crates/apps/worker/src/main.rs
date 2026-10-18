//! NAMU worker: the data plane. Workers drive contexts through the workflow IR themselves,
//! execute the tasks whose runtime they support, and hand other work to each other through
//! Redis streams. The orchestrator is not involved per step.

// `#[async_trait]` expansions trip this lint on recent clippy; the code itself has no
// `#[must_use]`.
#![allow(clippy::double_must_use)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use namu_proto::{
    TaskRuntime, WorkItem, WorkerRegisterRequest, WorkerRegisterResponse, host_platform, keys,
};
use redis::aio::ConnectionManager;
use redis::streams::{
    StreamAutoClaimOptions, StreamAutoClaimReply, StreamClaimOptions, StreamId,
    StreamPendingCountReply, StreamReadOptions, StreamReadReply,
};
use redis::{AsyncCommands, RedisResult};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{error, info};

mod commit;
mod exec;
mod runtime;
mod tree;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let orchestrator_url = std::env::var("NAMU_ORCH_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string())
        .trim_end_matches('/')
        .to_string();
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
    let platform = host_platform();
    let worker_id = std::env::var("WORKER_ID").unwrap_or_else(|_| {
        format!(
            "{platform}-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        )
    });
    let runtimes = parse_runtimes(
        &std::env::var("NAMU_RUNTIMES").unwrap_or_else(|_| "wasm,native".to_string()),
    )?;
    let runtime_keys: HashSet<String> = runtimes
        .iter()
        .map(|rt| match rt {
            TaskRuntime::Wasm => "wasm".to_string(),
            TaskRuntime::Native => format!("native:{platform}"),
        })
        .collect();
    let concurrency = std::env::var("NAMU_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    let cache_dir = PathBuf::from(
        std::env::var("ARTIFACT_CACHE").unwrap_or_else(|_| "./data/cache".to_string()),
    )
    .join(&worker_id);
    tokio::fs::create_dir_all(&cache_dir).await?;

    let http = reqwest::Client::new();
    let registration: WorkerRegisterResponse = http
        .post(format!("{orchestrator_url}/workers/register"))
        .json(&WorkerRegisterRequest {
            worker_id: worker_id.clone(),
            runtimes: runtime_keys.iter().cloned().collect(),
            platform: platform.clone(),
        })
        .send()
        .await
        .context("register with orchestrator")?
        .error_for_status()?
        .json()
        .await?;

    let redis = ConnectionManager::new(redis::Client::open(redis_url.as_str())?).await?;
    // Blocking stream reads get their own connection so they never delay other commands.
    let mut reader = ConnectionManager::new(redis::Client::open(redis_url.as_str())?).await?;

    let mut queues = vec![keys::QUEUE_ANY.to_string()];
    queues.extend(runtime_keys.iter().map(|key| keys::queue_runtime(key)));
    for queue in &queues {
        ensure_group(&mut reader, queue).await?;
    }

    let writer = tree::Writer::spawn(redis.clone());
    let ids = Arc::new(tree::IdMinter::new(registration.worker_index));
    let executor = Arc::new(exec::Executor::new(
        worker_id.clone(),
        runtime_keys.clone(),
        redis.clone(),
        writer,
        ids,
        runtime::Runtimes::new(http, orchestrator_url, cache_dir),
    ));
    info!(
        worker_id,
        worker_index = registration.worker_index,
        ?runtime_keys,
        concurrency,
        "worker ready"
    );

    let lease_ms = std::env::var("NAMU_LEASE_MS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(30_000);
    let shared = Arc::new(Shared {
        executor,
        redis: redis.clone(),
        permits: Arc::new(Semaphore::new(concurrency)),
        inflight: Mutex::new(HashMap::new()),
        worker_id: worker_id.clone(),
    });
    tokio::spawn(renew_leases(shared.clone(), lease_ms));
    tokio::spawn(reclaim(shared.clone(), queues.clone(), lease_ms));

    let ids_start: Vec<&str> = queues.iter().map(|_| ">").collect();
    loop {
        // Wait for a free slot before taking work, so unprocessed items stay in the queue for
        // other workers.
        let first_permit = shared.permits.clone().acquire_owned().await?;
        let options = StreamReadOptions::default()
            .group(keys::CONSUMER_GROUP, &worker_id)
            .count(1)
            .block(1000);
        let reply: RedisResult<StreamReadReply> =
            reader.xread_options(&queues, &ids_start, &options).await;
        let reply = match reply {
            Ok(reply) => reply,
            Err(err) => {
                error!("queue read failed: {err}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        // COUNT applies per stream, so one read can return an item from every queue. Each one is
        // now pending for this worker and must be handled.
        let mut first_permit = Some(first_permit);
        for (queue, id, item) in items(reply) {
            let permit = match first_permit.take() {
                Some(permit) => permit,
                None => shared.permits.clone().acquire_owned().await?,
            };
            dispatch(shared.clone(), permit, queue, id, item, 1);
        }
    }
}

struct Shared {
    executor: Arc<exec::Executor>,
    redis: ConnectionManager,
    permits: Arc<Semaphore>,
    /// Deliveries being processed here, per queue; their leases are renewed.
    inflight: Mutex<HashMap<String, HashSet<String>>>,
    worker_id: String,
}

/// Processes a delivery in the background. Success commits and acks it; failure leaves it
/// pending, to be reclaimed and retried once its lease expires.
fn dispatch(
    shared: Arc<Shared>,
    permit: OwnedSemaphorePermit,
    queue: String,
    id: String,
    item: anyhow::Result<WorkItem>,
    attempts: usize,
) {
    tokio::spawn(async move {
        let _permit = permit;
        let item = match item {
            Ok(item) => item,
            Err(err) => {
                // Unreadable: no attempt can succeed, so drop it.
                error!("bad work item {queue}/{id}: {err:#}");
                let mut redis = shared.redis.clone();
                let _: RedisResult<()> = redis.xack(&queue, keys::CONSUMER_GROUP, &[&id]).await;
                return;
            }
        };
        shared
            .inflight
            .lock()
            .unwrap()
            .entry(queue.clone())
            .or_default()
            .insert(id.clone());
        let delivery = commit::Delivery {
            queue: queue.clone(),
            id: id.clone(),
            attempts,
        };
        if let Err(err) = shared.executor.handle(item, &delivery).await {
            error!("work item {queue}/{id} failed (attempt {attempts}), will retry: {err:#}");
        }
        if let Some(ids) = shared.inflight.lock().unwrap().get_mut(&queue) {
            ids.remove(&id);
        }
    });
}

/// Keeps the leases of in-flight deliveries fresh so other workers don't reclaim them.
async fn renew_leases(shared: Arc<Shared>, lease_ms: usize) {
    let mut interval = tokio::time::interval(Duration::from_millis((lease_ms / 3).max(100) as u64));
    loop {
        interval.tick().await;
        let inflight: Vec<(String, Vec<String>)> = shared
            .inflight
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, ids)| !ids.is_empty())
            .map(|(queue, ids)| (queue.clone(), ids.iter().cloned().collect()))
            .collect();
        let mut redis = shared.redis.clone();
        for (queue, ids) in inflight {
            // Claiming an entry we own resets its idle time.
            let renewed: RedisResult<redis::Value> = redis
                .xclaim_options(
                    &queue,
                    keys::CONSUMER_GROUP,
                    &shared.worker_id,
                    0,
                    &ids,
                    StreamClaimOptions::default().with_justid(),
                )
                .await;
            if let Err(err) = renewed {
                error!("lease renewal failed: {err}");
            }
        }
    }
}

/// Takes over deliveries whose lease expired: their worker crashed, hung, or gave up.
async fn reclaim(shared: Arc<Shared>, queues: Vec<String>, lease_ms: usize) {
    let mut interval = tokio::time::interval(Duration::from_millis(
        (lease_ms / 4).clamp(100, 5_000) as u64,
    ));
    loop {
        interval.tick().await;
        let mut redis = shared.redis.clone();
        for queue in &queues {
            let claimed: RedisResult<StreamAutoClaimReply> = redis
                .xautoclaim_options(
                    queue,
                    keys::CONSUMER_GROUP,
                    &shared.worker_id,
                    lease_ms,
                    "0-0",
                    StreamAutoClaimOptions::default().count(16),
                )
                .await;
            let claimed = match claimed {
                Ok(claimed) => claimed.claimed,
                Err(err) => {
                    error!("reclaim failed: {err}");
                    continue;
                }
            };
            for entry in claimed {
                let pending: RedisResult<StreamPendingCountReply> = redis
                    .xpending_count(queue, keys::CONSUMER_GROUP, &entry.id, &entry.id, 1)
                    .await;
                let attempts = pending
                    .ok()
                    .and_then(|p| p.ids.first().map(|id| id.times_delivered))
                    .unwrap_or(1);
                info!(
                    queue,
                    id = entry.id,
                    attempts,
                    "reclaimed expired work item"
                );
                let item = parse_item(&entry);
                let Ok(permit) = shared.permits.clone().acquire_owned().await else {
                    return;
                };
                dispatch(
                    shared.clone(),
                    permit,
                    queue.clone(),
                    entry.id.clone(),
                    item,
                    attempts,
                );
            }
        }
    }
}

fn parse_runtimes(raw: &str) -> anyhow::Result<Vec<TaskRuntime>> {
    raw.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| TaskRuntime::parse(s).with_context(|| format!("unknown runtime {s:?}")))
        .collect()
}

async fn ensure_group(redis: &mut ConnectionManager, queue: &str) -> anyhow::Result<()> {
    let result: RedisResult<()> = redis
        .xgroup_create_mkstream(queue, keys::CONSUMER_GROUP, "0")
        .await;
    match result {
        Err(err) if !err.to_string().contains("BUSYGROUP") => Err(err.into()),
        _ => Ok(()),
    }
}

fn parse_item(entry: &StreamId) -> anyhow::Result<WorkItem> {
    let raw = entry
        .get::<String>("item")
        .context("work item without payload")?;
    Ok(serde_json::from_str::<WorkItem>(&raw)?)
}

fn items(reply: StreamReadReply) -> Vec<(String, String, anyhow::Result<WorkItem>)> {
    let mut out = Vec::new();
    for stream in reply.keys {
        for entry in stream.ids {
            let item = parse_item(&entry);
            out.push((stream.key.clone(), entry.id, item));
        }
    }
    out
}
