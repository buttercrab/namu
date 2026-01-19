use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use libloading::Library;
use namu_proto::{QueueMessage, TaskCompleteRequest, TaskManifest, TaskRuntime, TaskStartRequest};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde_json::Value as JsonValue;
use tracing::{error, info};
use uuid::Uuid;

mod object_store;
mod wasm_executor;

const MAX_PARENT_HOPS: usize = 10_000;

struct CachedValue {
    value: JsonValue,
    bytes: usize,
    tick: u64,
}

struct ValueCache {
    max_bytes: usize,
    current_bytes: usize,
    counter: u64,
    entries: HashMap<String, CachedValue>,
    order: VecDeque<(String, u64)>,
}

impl ValueCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            current_bytes: 0,
            counter: 0,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&mut self, hash: &str) -> Option<JsonValue> {
        let entry = self.entries.get_mut(hash)?;
        self.counter = self.counter.wrapping_add(1);
        entry.tick = self.counter;
        self.order.push_back((hash.to_string(), entry.tick));
        Some(entry.value.clone())
    }

    fn insert(&mut self, hash: String, value: JsonValue, bytes: usize) {
        if self.max_bytes == 0 || bytes > self.max_bytes {
            return;
        }
        if let Some(existing) = self.entries.remove(&hash) {
            self.current_bytes = self.current_bytes.saturating_sub(existing.bytes);
        }
        self.counter = self.counter.wrapping_add(1);
        self.entries.insert(
            hash.clone(),
            CachedValue {
                value,
                bytes,
                tick: self.counter,
            },
        );
        self.order.push_back((hash, self.counter));
        self.current_bytes = self.current_bytes.saturating_add(bytes);

        while self.current_bytes > self.max_bytes {
            let Some((key, tick)) = self.order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.get(&key)
                && entry.tick != tick
            {
                continue;
            }
            if let Some(removed) = self.entries.remove(&key) {
                self.current_bytes = self.current_bytes.saturating_sub(removed.bytes);
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let orchestrator_url =
        std::env::var("NAMU_ORCH_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
    let worker_id = std::env::var("WORKER_ID").unwrap_or_else(|_| Uuid::new_v4().to_string());
    let resource_class =
        std::env::var("RESOURCE_CLASS").unwrap_or_else(|_| "cpu.small".to_string());
    let worker_pool =
        normalize_pool(&std::env::var("WORKER_POOL").unwrap_or_else(|_| "trusted".to_string()))?;
    let labels_json = std::env::var("LABELS_JSON").unwrap_or_else(|_| "{}".to_string());
    let cache_dir = std::env::var("ARTIFACT_CACHE").unwrap_or_else(|_| "./data/cache".to_string());
    let value_cache_bytes = std::env::var("NAMU_VALUE_CACHE_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(268_435_456);
    let object_store = object_store::ObjectStore::from_env().await?;

    let client = reqwest::Client::new();
    register_worker(
        &client,
        &orchestrator_url,
        &worker_id,
        &worker_pool,
        &resource_class,
        &labels_json,
    )
    .await?;

    let redis_client = redis::Client::open(redis_url)?;
    let mut redis = ConnectionManager::new(redis_client).await?;

    let stream = format!("queue:{worker_pool}:{resource_class}");
    let group = format!("workers:{worker_pool}:{resource_class}");
    ensure_group(&mut redis, &stream, &group).await?;

    let cache_dir = PathBuf::from(cache_dir);
    tokio::fs::create_dir_all(&cache_dir).await?;
    let mut value_cache = ValueCache::new(value_cache_bytes);

    loop {
        match read_one(&mut redis, &stream, &group, &worker_id).await {
            Ok(Some((message_id, payload))) => {
                if let Err(err) = handle_message(
                    &client,
                    &orchestrator_url,
                    &mut redis,
                    &cache_dir,
                    &mut value_cache,
                    object_store.as_ref(),
                    &payload,
                )
                .await
                {
                    error!("task failed: {err}");
                }
                let _: () = redis::cmd("XACK")
                    .arg(&stream)
                    .arg(&group)
                    .arg(&message_id)
                    .query_async(&mut redis)
                    .await?;
            }
            Ok(None) => continue,
            Err(err) => {
                error!("redis error: {err}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn register_worker(
    client: &reqwest::Client,
    orchestrator_url: &str,
    worker_id: &str,
    worker_pool: &str,
    resource_class: &str,
    labels_json: &str,
) -> anyhow::Result<()> {
    let labels: JsonValue = serde_json::from_str(labels_json)?;
    client
        .post(format!("{orchestrator_url}/workers/register"))
        .json(&serde_json::json!({
            "worker_id": worker_id,
            "pool": worker_pool,
            "resource_class": resource_class,
            "labels": labels
        }))
        .send()
        .await?
        .error_for_status()?;
    info!("worker registered: {worker_id}");
    Ok(())
}

fn normalize_pool(value: &str) -> anyhow::Result<String> {
    let pool = value.trim().to_ascii_lowercase();
    match pool.as_str() {
        "trusted" | "restricted" | "wasm" | "gpu" => Ok(pool),
        _ => Err(anyhow::anyhow!(
            "invalid WORKER_POOL '{value}' (expected trusted|restricted|wasm|gpu)"
        )),
    }
}

async fn ensure_group(
    redis: &mut ConnectionManager,
    stream: &str,
    group: &str,
) -> anyhow::Result<()> {
    let result: redis::RedisResult<()> = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(stream)
        .arg(group)
        .arg("0")
        .arg("MKSTREAM")
        .query_async(redis)
        .await;
    if let Err(err) = result {
        let msg = err.to_string();
        if !msg.contains("BUSYGROUP") {
            return Err(err.into());
        }
    }
    Ok(())
}

async fn read_one(
    redis: &mut ConnectionManager,
    stream: &str,
    group: &str,
    consumer: &str,
) -> anyhow::Result<Option<(String, QueueMessage)>> {
    let reply: redis::Value = redis::cmd("XREADGROUP")
        .arg("GROUP")
        .arg(group)
        .arg(consumer)
        .arg("COUNT")
        .arg(1)
        .arg("BLOCK")
        .arg(5000)
        .arg("STREAMS")
        .arg(stream)
        .arg(">")
        .query_async(redis)
        .await?;

    parse_queue_reply(reply)
}

fn parse_queue_reply(value: redis::Value) -> anyhow::Result<Option<(String, QueueMessage)>> {
    let redis::Value::Array(streams) = value else {
        return Ok(None);
    };
    for stream in streams {
        let redis::Value::Array(stream_parts) = stream else {
            continue;
        };
        if stream_parts.len() != 2 {
            continue;
        }
        let redis::Value::Array(entries) = &stream_parts[1] else {
            continue;
        };
        for entry in entries {
            let redis::Value::Array(entry_parts) = entry else {
                continue;
            };
            if entry_parts.len() != 2 {
                continue;
            }
            let id = redis::from_redis_value::<String>(&entry_parts[0])?;
            let redis::Value::Array(kv) = &entry_parts[1] else {
                continue;
            };
            let mut i = 0;
            while i + 1 < kv.len() {
                let field = redis::from_redis_value::<String>(&kv[i])?;
                let value = &kv[i + 1];
                if field == "payload" {
                    let payload = redis::from_redis_value::<String>(value)?;
                    let msg: QueueMessage = serde_json::from_str(&payload)?;
                    return Ok(Some((id, msg)));
                }
                i += 2;
            }
        }
    }
    Ok(None)
}

async fn handle_message(
    client: &reqwest::Client,
    orchestrator_url: &str,
    redis: &mut ConnectionManager,
    cache_dir: &Path,
    value_cache: &mut ValueCache,
    object_store: Option<&object_store::ObjectStore>,
    msg: &QueueMessage,
) -> anyhow::Result<()> {
    start_task(client, orchestrator_url, msg).await?;

    let manifest = fetch_manifest(client, orchestrator_url, &msg.task_id, &msg.task_version)
        .await
        .context("fetch manifest")?;

    let artifact_path = ensure_artifact(
        client,
        orchestrator_url,
        cache_dir,
        &msg.task_id,
        &msg.task_version,
        &manifest.runtime,
    )
    .await?;

    let inputs = resolve_inputs(redis, value_cache, object_store, msg).await?;
    let input_json = build_input_json(&manifest, inputs)?;

    match call_task(&manifest.runtime, &artifact_path, &input_json)? {
        Ok(output) => {
            complete_task(client, orchestrator_url, msg, true, Some(output), None).await?;
        }
        Err(err_json) => {
            complete_task(client, orchestrator_url, msg, false, None, Some(err_json)).await?;
        }
    }

    Ok(())
}

async fn start_task(
    client: &reqwest::Client,
    orchestrator_url: &str,
    msg: &QueueMessage,
) -> anyhow::Result<()> {
    let req = TaskStartRequest {
        op_id: msg.op_id,
        ctx_id: msg.ctx_id,
        lease_ms: msg.lease_ms,
    };
    client
        .post(format!(
            "{orchestrator_url}/runs/{}/nodes/start",
            msg.run_id
        ))
        .json(&req)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn complete_task(
    client: &reqwest::Client,
    orchestrator_url: &str,
    msg: &QueueMessage,
    success: bool,
    output: Option<JsonValue>,
    error: Option<String>,
) -> anyhow::Result<()> {
    let req = TaskCompleteRequest {
        op_id: msg.op_id,
        ctx_id: msg.ctx_id,
        success,
        output_json: output,
        error,
    };
    client
        .post(format!(
            "{orchestrator_url}/runs/{}/nodes/complete",
            msg.run_id
        ))
        .json(&req)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn fetch_manifest(
    client: &reqwest::Client,
    orchestrator_url: &str,
    task_id: &str,
    version: &str,
) -> anyhow::Result<TaskManifest> {
    let resp = client
        .get(format!("{orchestrator_url}/tasks/{task_id}/{version}"))
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json::<TaskManifest>().await?)
}

async fn ensure_artifact(
    client: &reqwest::Client,
    orchestrator_url: &str,
    cache_dir: &Path,
    task_id: &str,
    version: &str,
    runtime: &TaskRuntime,
) -> anyhow::Result<PathBuf> {
    let dir = cache_dir.join(task_id).join(version);
    tokio::fs::create_dir_all(&dir).await?;
    let archive_path = dir.join("artifact.tar.zst");
    if archive_path.exists() {
        return extract_artifact(&archive_path, &dir, runtime).await;
    }

    let bytes = client
        .get(format!(
            "{orchestrator_url}/tasks/{task_id}/{version}/artifact"
        ))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    tokio::fs::write(&archive_path, &bytes).await?;
    extract_artifact(&archive_path, &dir, runtime).await
}

async fn extract_artifact(
    archive_path: &Path,
    dir: &Path,
    runtime: &TaskRuntime,
) -> anyhow::Result<PathBuf> {
    let data = tokio::fs::read(archive_path).await?;
    let mut decoder = zstd::stream::read::Decoder::new(std::io::Cursor::new(data))?;
    let mut archive = tar::Archive::new(&mut decoder);
    let mut artifact_path = None;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        if path.file_name().is_some() {
            let name = path.file_name().unwrap().to_string_lossy();
            let is_match = match runtime {
                TaskRuntime::Native => {
                    name.ends_with(".so") || name.ends_with(".dylib") || name.ends_with(".dll")
                }
                TaskRuntime::Wasm => name.ends_with(".wasm"),
            };
            if is_match {
                let out_path = dir.join(name.as_ref());
                entry.unpack(&out_path)?;
                artifact_path = Some(out_path);
            }
        }
    }

    let kind = match runtime {
        TaskRuntime::Native => "native library",
        TaskRuntime::Wasm => "wasm module",
    };
    artifact_path.ok_or_else(|| anyhow::anyhow!("{kind} not found in artifact"))
}

async fn resolve_inputs(
    redis: &mut ConnectionManager,
    value_cache: &mut ValueCache,
    object_store: Option<&object_store::ObjectStore>,
    msg: &QueueMessage,
) -> anyhow::Result<Vec<JsonValue>> {
    let mut inputs = Vec::with_capacity(msg.input_ids.len());
    let hashes = msg.input_hashes.as_ref();
    let inline = msg.input_values.as_ref();
    let refs = msg.input_refs.as_ref();

    for (idx, id) in msg.input_ids.iter().enumerate() {
        if let Some(hashes) = hashes
            && let Some(hash) = hashes.get(idx)
            && let Some(val) = value_cache.get(hash)
        {
            inputs.push(val);
            continue;
        }

        if let Some(inline_vals) = inline
            && let Some(val) = inline_vals.get(idx)
        {
            let value = val.clone();
            if let Some(hashes) = hashes
                && let Some(hash) = hashes.get(idx)
            {
                let bytes = estimate_json_size(&value);
                value_cache.insert(hash.clone(), value.clone(), bytes);
            }
            inputs.push(value);
            continue;
        }

        if let Some(refs) = refs
            && let Some(Some(value_ref)) = refs.get(idx)
            && let Some(store) = object_store
        {
            match store.get_json(value_ref).await {
                Ok(bytes) => {
                    let value: JsonValue = serde_json::from_slice(&bytes)?;
                    if let Some(hashes) = hashes
                        && let Some(hash) = hashes.get(idx)
                    {
                        value_cache.insert(hash.clone(), value.clone(), bytes.len());
                    }
                    inputs.push(value);
                    continue;
                }
                Err(err) => {
                    error!("object store fetch failed: {err}");
                }
            }
        }

        let raw = get_value_raw(redis, msg.run_id, msg.ctx_id, *id).await?;
        let raw = raw.ok_or_else(|| anyhow::anyhow!("missing input value"))?;
        let value: JsonValue = serde_json::from_str(&raw)?;
        if let Some(hashes) = hashes
            && let Some(hash) = hashes.get(idx)
        {
            value_cache.insert(hash.clone(), value.clone(), raw.len());
        }
        inputs.push(value);
    }
    Ok(inputs)
}

fn estimate_json_size(value: &JsonValue) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn context_key(run_id: Uuid, ctx_id: usize) -> String {
    format!("context:{run_id}:{ctx_id}")
}

async fn get_parent(
    redis: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
) -> anyhow::Result<Option<usize>> {
    let key = context_key(run_id, ctx_id);
    let raw: Option<String> = redis.hget(key, "parent").await?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_empty() || raw == "-1" {
        return Ok(None);
    }
    let parent = raw
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("invalid parent ctx id {raw}"))?;
    Ok(Some(parent))
}

async fn get_value_raw(
    redis: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
    value_id: usize,
) -> anyhow::Result<Option<String>> {
    let field = value_id.to_string();
    let mut current_ctx = Some(ctx_id);
    let mut hops = 0usize;
    while let Some(ctx) = current_ctx {
        let key = format!("values:{run_id}:{ctx}");
        if let Some(payload) = redis.hget::<_, _, Option<String>>(key, &field).await? {
            return Ok(Some(payload));
        }
        current_ctx = get_parent(redis, run_id, ctx).await?;
        hops = hops.saturating_add(1);
        if hops > MAX_PARENT_HOPS {
            return Err(anyhow::anyhow!(
                "context parent chain exceeded {MAX_PARENT_HOPS} hops"
            ));
        }
    }
    Ok(None)
}

fn build_input_json(manifest: &TaskManifest, inputs: Vec<JsonValue>) -> anyhow::Result<JsonValue> {
    if manifest.input_arity == 1 {
        Ok(inputs.into_iter().next().unwrap_or(JsonValue::Null))
    } else {
        Ok(JsonValue::Array(inputs))
    }
}

fn call_task(
    runtime: &TaskRuntime,
    artifact_path: &Path,
    input_json: &JsonValue,
) -> anyhow::Result<Result<JsonValue, String>> {
    match runtime {
        TaskRuntime::Native => call_task_native(artifact_path, input_json),
        TaskRuntime::Wasm => wasm_executor::call_task(artifact_path, input_json),
    }
}

fn call_task_native(
    lib_path: &Path,
    input_json: &JsonValue,
) -> anyhow::Result<Result<JsonValue, String>> {
    unsafe {
        let lib = Library::new(lib_path)?;
        let create: libloading::Symbol<unsafe extern "C" fn() -> *mut std::ffi::c_void> =
            lib.get(b"namu_task_create")?;
        let destroy: libloading::Symbol<unsafe extern "C" fn(*mut std::ffi::c_void)> =
            lib.get(b"namu_task_destroy")?;
        let call: libloading::Symbol<
            unsafe extern "C" fn(
                *mut std::ffi::c_void,
                *const u8,
                usize,
                *mut u8,
                *mut usize,
            ) -> i32,
        > = lib.get(b"namu_task_call")?;

        let handle = create();
        let input_bytes = serde_json::to_vec(input_json)?;
        let mut output_len: usize = 4096;
        let mut buffer = vec![0u8; output_len];
        let mut code = call(
            handle,
            input_bytes.as_ptr(),
            input_bytes.len(),
            buffer.as_mut_ptr(),
            &mut output_len as *mut usize,
        );

        if code != 0 && output_len > buffer.len() {
            buffer.resize(output_len, 0u8);
            code = call(
                handle,
                input_bytes.as_ptr(),
                input_bytes.len(),
                buffer.as_mut_ptr(),
                &mut output_len as *mut usize,
            );
        }

        destroy(handle);

        let output_bytes = &buffer[..output_len.min(buffer.len())];
        let output_str = std::str::from_utf8(output_bytes).unwrap_or("");
        if code == 0 {
            let json = serde_json::from_str(output_str)?;
            Ok(Ok(json))
        } else {
            let err = if output_str.is_empty() {
                "task failed".to_string()
            } else {
                output_str.to_string()
            };
            Ok(Err(err))
        }
    }
}
