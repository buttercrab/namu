//! NAMU worker: the data plane. Workers drive contexts through the workflow IR themselves,
//! execute the tasks whose runtime they support, and hand other work to each other through
//! Redis streams. The orchestrator is not involved per step.

// `#[async_trait]` expansions trip this lint on recent clippy; the code itself has no
// `#[must_use]`.
#![allow(clippy::double_must_use)]

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use namu_proto::{
    TaskRuntime, WorkItem, WorkerRegisterRequest, WorkerRegisterResponse, host_platform, keys,
};
use redis::aio::ConnectionManager;
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::{AsyncCommands, RedisResult};
use tokio::sync::Semaphore;
use tracing::{error, info};

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

    let permits = Arc::new(Semaphore::new(concurrency));
    let ids_start: Vec<&str> = queues.iter().map(|_| ">").collect();
    loop {
        // Wait for a free slot before taking work, so unprocessed items stay in the queue for
        // other workers.
        let first_permit = permits.clone().acquire_owned().await?;
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
        // now pending for this worker and must be handled and acked.
        let mut first_permit = Some(first_permit);
        for (queue, id, item) in items(reply) {
            let permit = match first_permit.take() {
                Some(permit) => permit,
                None => permits.clone().acquire_owned().await?,
            };
            let executor = executor.clone();
            let mut redis = redis.clone();
            tokio::spawn(async move {
                let _permit = permit;
                match item {
                    Ok(item) => {
                        if let Err(err) = executor.handle(item).await {
                            error!("work item failed: {err:#}");
                        }
                    }
                    Err(err) => error!("bad work item: {err:#}"),
                }
                let acked: RedisResult<()> = redis.xack(&queue, keys::CONSUMER_GROUP, &[&id]).await;
                if let Err(err) = acked {
                    error!("ack failed: {err}");
                }
            });
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

fn items(reply: StreamReadReply) -> Vec<(String, String, anyhow::Result<WorkItem>)> {
    let mut out = Vec::new();
    for stream in reply.keys {
        for entry in stream.ids {
            let item = entry
                .get::<String>("item")
                .context("work item without payload")
                .and_then(|raw| Ok(serde_json::from_str::<WorkItem>(&raw)?));
            out.push((stream.key.clone(), entry.id, item));
        }
    }
    out
}
