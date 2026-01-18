use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use libloading::Library;
use namu_proto::{QueueMessage, TaskCompleteRequest, TaskManifest, TaskStartRequest};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde_json::Value as JsonValue;
use tracing::{error, info};
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let orchestrator_url =
        std::env::var("NAMU_ORCH_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
    let worker_id = std::env::var("WORKER_ID").unwrap_or_else(|_| Uuid::new_v4().to_string());
    let resource_class =
        std::env::var("RESOURCE_CLASS").unwrap_or_else(|_| "cpu.small".to_string());
    let labels_json = std::env::var("LABELS_JSON").unwrap_or_else(|_| "{}".to_string());
    let cache_dir = std::env::var("ARTIFACT_CACHE").unwrap_or_else(|_| "./data/cache".to_string());

    let client = reqwest::Client::new();
    register_worker(
        &client,
        &orchestrator_url,
        &worker_id,
        &resource_class,
        &labels_json,
    )
    .await?;

    let redis_client = redis::Client::open(redis_url)?;
    let mut redis = ConnectionManager::new(redis_client).await?;

    let stream = format!("queue:{resource_class}");
    let group = format!("workers:{resource_class}");
    ensure_group(&mut redis, &stream, &group).await?;

    let cache_dir = PathBuf::from(cache_dir);
    tokio::fs::create_dir_all(&cache_dir).await?;

    loop {
        match read_one(&mut redis, &stream, &group, &worker_id).await {
            Ok(Some((message_id, payload))) => {
                if let Err(err) =
                    handle_message(&client, &orchestrator_url, &mut redis, &cache_dir, &payload)
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
    resource_class: &str,
    labels_json: &str,
) -> anyhow::Result<()> {
    let labels: JsonValue = serde_json::from_str(labels_json)?;
    client
        .post(format!("{orchestrator_url}/workers/register"))
        .json(&serde_json::json!({
            "worker_id": worker_id,
            "resource_class": resource_class,
            "labels": labels
        }))
        .send()
        .await?
        .error_for_status()?;
    info!("worker registered: {worker_id}");
    Ok(())
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
    msg: &QueueMessage,
) -> anyhow::Result<()> {
    start_task(client, orchestrator_url, msg).await?;

    let manifest = fetch_manifest(client, orchestrator_url, &msg.task_id, &msg.task_version)
        .await
        .context("fetch manifest")?;

    let lib_path = ensure_artifact(
        client,
        orchestrator_url,
        cache_dir,
        &msg.task_id,
        &msg.task_version,
    )
    .await?;

    let inputs = fetch_inputs(redis, msg).await?;
    let input_json = build_input_json(&manifest, inputs)?;

    match call_task(&lib_path, &input_json)? {
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
) -> anyhow::Result<PathBuf> {
    let dir = cache_dir.join(task_id).join(version);
    tokio::fs::create_dir_all(&dir).await?;
    let archive_path = dir.join("artifact.tar.zst");
    if archive_path.exists() {
        return extract_library(&archive_path, &dir).await;
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
    extract_library(&archive_path, &dir).await
}

async fn extract_library(archive_path: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    let data = tokio::fs::read(archive_path).await?;
    let mut decoder = zstd::stream::read::Decoder::new(std::io::Cursor::new(data))?;
    let mut archive = tar::Archive::new(&mut decoder);
    let mut lib_path = None;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        if path.file_name().is_some() {
            let name = path.file_name().unwrap().to_string_lossy();
            if name.ends_with(".so") || name.ends_with(".dylib") || name.ends_with(".dll") {
                let out_path = dir.join(name.as_ref());
                entry.unpack(&out_path)?;
                lib_path = Some(out_path);
            }
        }
    }

    lib_path.ok_or_else(|| anyhow::anyhow!("library not found in artifact"))
}

async fn fetch_inputs(
    redis: &mut ConnectionManager,
    msg: &QueueMessage,
) -> anyhow::Result<Vec<JsonValue>> {
    let key = format!("values:{}:{}", msg.run_id, msg.ctx_id);
    let mut inputs = Vec::with_capacity(msg.input_ids.len());
    for id in &msg.input_ids {
        let raw: Option<String> = redis.hget(&key, id.to_string()).await?;
        let raw = raw.ok_or_else(|| anyhow::anyhow!("missing input value"))?;
        inputs.push(serde_json::from_str(&raw)?);
    }
    Ok(inputs)
}

fn build_input_json(manifest: &TaskManifest, inputs: Vec<JsonValue>) -> anyhow::Result<JsonValue> {
    if manifest.input_arity == 1 {
        Ok(inputs.into_iter().next().unwrap_or(JsonValue::Null))
    } else {
        Ok(JsonValue::Array(inputs))
    }
}

fn call_task(lib_path: &Path, input_json: &JsonValue) -> anyhow::Result<Result<JsonValue, String>> {
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
