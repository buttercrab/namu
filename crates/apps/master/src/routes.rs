use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use bytes::Bytes;
use namu_core::ir::Workflow;
use namu_engine::RunOutcome;
use namu_engine::message::Message;
use namu_engine::transport::Transport;
use namu_proto::{
    HeartbeatResponse, MEMBERSHIP_TTL_MS, Member, RunCreateRequest, RunCreateResponse,
    RunStatusResponse, TaskManifest, WorkerRegisterRequest, WorkerRegisterResponse,
    WorkflowUploadRequest, keys,
};
use redis::AsyncCommands;
use serde_json::{Value as JsonValue, json};
use sha2::Digest;
use uuid::Uuid;

use crate::{AppState, storage};

type ApiResult<T> = Result<T, (StatusCode, String)>;

fn bad_request(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, err.to_string())
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!("{err}");
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub async fn healthz() -> Json<JsonValue> {
    Json(json!({"status": "ok"}))
}

/// Publishes a task version. Versions are immutable: re-publishing the identical artifact is a
/// no-op, a different artifact under the same version is rejected. (Lineage recovery re-runs the
/// exact version a run used, so it must never change.)
pub async fn upload_task(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> ApiResult<Json<JsonValue>> {
    let mut artifact: Option<Bytes> = None;
    while let Some(field) = multipart.next_field().await.map_err(bad_request)? {
        if field.name() == Some("artifact") {
            artifact = Some(field.bytes().await.map_err(bad_request)?);
        }
    }
    let bytes = artifact.ok_or_else(|| bad_request("missing `artifact` field"))?;
    let mut manifest = read_artifact(&bytes).map_err(bad_request)?;
    storage::safe_segment(&manifest.task_id).map_err(bad_request)?;
    storage::safe_segment(&manifest.version).map_err(bad_request)?;

    let key = keys::task(&manifest.task_id, &manifest.version);
    let mut redis = state.redis.clone();
    let existing: Option<String> = redis.get(&key).await.map_err(internal)?;
    if let Some(existing) = existing {
        let existing: TaskManifest = serde_json::from_str(&existing).map_err(internal)?;
        // Workers place and invoke tasks from the manifest, so metadata changes (runtime,
        // platform, arity, ...) count as a different version just like a different binary.
        if comparable(&existing).map_err(internal)? == comparable(&manifest).map_err(internal)? {
            return Ok(Json(
                json!({"task_id": manifest.task_id, "version": manifest.version, "unchanged": true}),
            ));
        }
        return Err((
            StatusCode::CONFLICT,
            format!(
                "{}@{} is already published with a different artifact or manifest; bump the version",
                manifest.task_id, manifest.version
            ),
        ));
    }

    storage::store_artifact(
        &state.artifacts_dir,
        &manifest.task_id,
        &manifest.version,
        &bytes,
    )
    .await
    .map_err(internal)?;
    manifest.created_at = now_ms().to_string();
    let _: () = redis
        .set(&key, serde_json::to_string(&manifest).map_err(internal)?)
        .await
        .map_err(internal)?;
    Ok(Json(
        json!({"task_id": manifest.task_id, "version": manifest.version}),
    ))
}

/// A manifest as JSON without fields the orchestrator sets itself.
fn comparable(manifest: &TaskManifest) -> serde_json::Result<JsonValue> {
    let mut value = serde_json::to_value(manifest)?;
    if let Some(fields) = value.as_object_mut() {
        fields.remove("created_at");
    }
    Ok(value)
}

/// Reads the manifest from an artifact and checks the binary against its checksum.
fn read_artifact(bytes: &Bytes) -> anyhow::Result<TaskManifest> {
    let decoder = zstd::stream::read::Decoder::new(Cursor::new(bytes))?;
    let mut archive = tar::Archive::new(decoder);
    let mut manifest: Option<TaskManifest> = None;
    let mut binary_digest: Option<String> = None;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry
            .path()?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        if name == "manifest.json" {
            manifest = Some(serde_json::from_slice(&buf)?);
        } else if [".wasm", ".so", ".dylib", ".dll"]
            .iter()
            .any(|ext| name.ends_with(ext))
        {
            binary_digest = Some(format!("sha256:{:x}", sha2::Sha256::digest(&buf)));
        }
    }
    let manifest = manifest.ok_or_else(|| anyhow::anyhow!("artifact has no manifest.json"))?;
    let digest = binary_digest.ok_or_else(|| anyhow::anyhow!("artifact has no task binary"))?;
    anyhow::ensure!(
        manifest.checksum == digest,
        "manifest checksum {} does not match binary {digest}",
        manifest.checksum
    );
    Ok(manifest)
}

pub async fn get_task_manifest(
    State(state): State<AppState>,
    Path((task_id, version)): Path<(String, String)>,
) -> ApiResult<Json<TaskManifest>> {
    let mut redis = state.redis.clone();
    let raw: Option<String> = redis
        .get(keys::task(&task_id, &version))
        .await
        .map_err(internal)?;
    let raw = raw.ok_or((StatusCode::NOT_FOUND, "unknown task".to_string()))?;
    Ok(Json(serde_json::from_str(&raw).map_err(internal)?))
}

pub async fn get_artifact(
    State(state): State<AppState>,
    Path((task_id, version)): Path<(String, String)>,
) -> ApiResult<Bytes> {
    storage::load_artifact(&state.artifacts_dir, &task_id, &version)
        .await
        .map_err(|err| (StatusCode::NOT_FOUND, err.to_string()))
}

/// Publishes a workflow version after checking that every task it calls is published.
pub async fn upload_workflow(
    State(state): State<AppState>,
    Json(req): Json<WorkflowUploadRequest>,
) -> ApiResult<Json<JsonValue>> {
    let workflow: Workflow = serde_json::from_value(req.ir.clone()).map_err(bad_request)?;
    let mut redis = state.redis.clone();
    let task_ids: BTreeSet<&str> = workflow
        .operations
        .iter()
        .filter_map(|op| op.call.as_ref().map(|c| c.task_id.as_str()))
        .collect();
    for task_id in task_ids {
        let version = req
            .task_versions
            .get(task_id)
            .ok_or_else(|| bad_request(format!("no version given for task {task_id}")))?;
        let exists: bool = redis
            .exists(keys::task(task_id, version))
            .await
            .map_err(internal)?;
        if !exists {
            return Err(bad_request(format!(
                "task {task_id}@{version} is not published"
            )));
        }
    }

    let key = keys::workflow(&req.id, &req.version);
    let payload = serde_json::to_string(&req).map_err(internal)?;
    let existing: Option<String> = redis.get(&key).await.map_err(internal)?;
    if let Some(existing) = existing {
        let existing: WorkflowUploadRequest = serde_json::from_str(&existing).map_err(internal)?;
        if existing.ir == req.ir && existing.task_versions == req.task_versions {
            return Ok(Json(
                json!({"id": req.id, "version": req.version, "unchanged": true}),
            ));
        }
        return Err((
            StatusCode::CONFLICT,
            format!(
                "workflow {}@{} already exists with different content; bump the version",
                req.id, req.version
            ),
        ));
    }
    let _: () = redis.set(&key, payload).await.map_err(internal)?;
    Ok(Json(json!({"id": req.id, "version": req.version})))
}

/// Starts a run: picks a worker to own it and sends it the workflow. From there the workers drive
/// the run; the owner reports the outcome when it is over.
pub async fn create_run(
    State(state): State<AppState>,
    Json(req): Json<RunCreateRequest>,
) -> ApiResult<Json<RunCreateResponse>> {
    let mut redis = state.redis.clone();
    let raw: Option<String> = redis
        .get(keys::workflow(&req.workflow_id, &req.version))
        .await
        .map_err(internal)?;
    let raw = raw.ok_or((StatusCode::NOT_FOUND, "unknown workflow".to_string()))?;
    let upload: WorkflowUploadRequest = serde_json::from_str(&raw).map_err(internal)?;
    let workflow: Workflow = serde_json::from_value(upload.ir).map_err(internal)?;

    let workers: Vec<Member> = members(&state)
        .await
        .map_err(internal)?
        .into_iter()
        .filter(|member| member.id != 0)
        .collect();
    if workers.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "no workers are registered".to_string(),
        ));
    }
    let cursor: u64 = redis.incr(keys::OWNER_CURSOR, 1).await.map_err(internal)?;
    let owner = &workers[cursor as usize % workers.len()];
    let owner_addr: SocketAddr = owner.addr.parse().map_err(internal)?;

    let run_id = Uuid::new_v4();
    let _: () = redis
        .hset_multiple(
            keys::run_state(run_id),
            &[
                ("status", "running".to_string()),
                ("created_at_ms", now_ms().to_string()),
            ],
        )
        .await
        .map_err(internal)?;
    state.mesh.set_peer(owner.id, owner_addr);
    state.mesh.send(
        owner.id,
        Message::Start {
            run: run_id,
            workflow,
            task_versions: upload.task_versions.into_iter().collect(),
            reply_to: 0,
        },
    );
    Ok(Json(RunCreateResponse { run_id }))
}

/// Stores a finished run's outcome, for status queries.
pub async fn record_outcome(
    state: &AppState,
    run: Uuid,
    outcome: RunOutcome,
) -> anyhow::Result<()> {
    let mut redis = state.redis.clone();
    let ids: Vec<u64> = outcome.workers.keys().copied().collect();
    let names: Vec<Option<String>> = if ids.is_empty() {
        Vec::new()
    } else {
        redis.hget(keys::WORKER_NAMES, &ids).await?
    };
    let status = if outcome.failures.is_empty() {
        "succeeded"
    } else {
        "partial_failed"
    };
    let mut pipe = redis::pipe();
    for result in &outcome.results {
        pipe.rpush(keys::run_results(run), serde_json::to_string(result)?)
            .ignore();
    }
    for failure in &outcome.failures {
        pipe.rpush(keys::run_failures(run), failure).ignore();
    }
    for ((id, stats), name) in outcome.workers.iter().zip(names) {
        let name = name.unwrap_or_else(|| format!("worker-{id}"));
        pipe.hset(keys::run_calls(run), &name, stats.calls).ignore();
        pipe.hset(keys::run_remote_fetches(run), &name, stats.fetches)
            .ignore();
    }
    pipe.hset_multiple(
        keys::run_state(run),
        &[
            ("status", status.to_string()),
            ("finished_at_ms", now_ms().to_string()),
        ],
    )
    .ignore();
    pipe.query_async::<()>(&mut redis).await?;
    Ok(())
}

pub async fn get_run_status(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> ApiResult<Json<RunStatusResponse>> {
    let mut redis = state.redis.clone();
    let run_state: BTreeMap<String, String> = redis
        .hgetall(keys::run_state(run_id))
        .await
        .map_err(internal)?;
    if run_state.is_empty() {
        return Err((StatusCode::NOT_FOUND, "unknown run".to_string()));
    }
    let results: Vec<String> = redis
        .lrange(keys::run_results(run_id), 0, -1)
        .await
        .map_err(internal)?;
    let failures: Vec<String> = redis
        .lrange(keys::run_failures(run_id), 0, -1)
        .await
        .map_err(internal)?;
    let calls: BTreeMap<String, u64> = redis
        .hgetall(keys::run_calls(run_id))
        .await
        .map_err(internal)?;
    let fetches: BTreeMap<String, u64> = redis
        .hgetall(keys::run_remote_fetches(run_id))
        .await
        .map_err(internal)?;

    let parse_ms = |field: &str| run_state.get(field).and_then(|v| v.parse::<u64>().ok());
    let elapsed_ms = match (parse_ms("created_at_ms"), parse_ms("finished_at_ms")) {
        (Some(start), Some(end)) => Some(end.saturating_sub(start)),
        (Some(start), None) => Some(now_ms().saturating_sub(start)),
        _ => None,
    };
    Ok(Json(RunStatusResponse {
        status: run_state
            .get("status")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        results: results
            .iter()
            .map(|r| serde_json::from_str(r).unwrap_or(JsonValue::String(r.clone())))
            .collect(),
        failures,
        elapsed_ms,
        calls_by_worker: calls,
        remote_node_fetches_by_worker: fetches,
    }))
}

/// Adds a worker to the mesh and gives it its id.
pub async fn register_worker(
    State(state): State<AppState>,
    Json(req): Json<WorkerRegisterRequest>,
) -> ApiResult<Json<WorkerRegisterResponse>> {
    let addr: SocketAddr = req.addr.parse().map_err(bad_request)?;
    let mut redis = state.redis.clone();
    // Id 0 is the orchestrator itself.
    let worker_id: u64 = redis.incr(keys::WORKER_SEQ, 1).await.map_err(internal)?;
    let member = Member {
        id: worker_id,
        name: req.name,
        addr: req.addr,
        runtimes: req.runtimes,
        platform: req.platform,
    };
    let _: () = redis::pipe()
        .set_ex(
            keys::member(worker_id),
            serde_json::to_string(&member).map_err(internal)?,
            MEMBERSHIP_TTL_MS / 1000,
        )
        .ignore()
        .hset(keys::WORKER_NAMES, worker_id, &member.name)
        .ignore()
        .query_async(&mut redis)
        .await
        .map_err(internal)?;
    state.mesh.set_peer(worker_id, addr);
    tracing::info!(worker_id, name = member.name, %addr, "worker joined");
    let members = members(&state).await.map_err(internal)?;
    Ok(Json(WorkerRegisterResponse { worker_id, members }))
}

/// Renews a worker's membership and returns the current members.
pub async fn heartbeat(
    State(state): State<AppState>,
    Path(worker_id): Path<u64>,
) -> ApiResult<Json<HeartbeatResponse>> {
    let mut redis = state.redis.clone();
    let ttl_ms = MEMBERSHIP_TTL_MS as i64;
    let renewed: bool = redis
        .pexpire(keys::member(worker_id), ttl_ms)
        .await
        .map_err(internal)?;
    if !renewed {
        return Err((
            StatusCode::GONE,
            format!("worker {worker_id} is no longer a member; register again"),
        ));
    }
    let members = members(&state).await.map_err(internal)?;
    Ok(Json(HeartbeatResponse { members }))
}

pub async fn list_workers(State(state): State<AppState>) -> ApiResult<Json<JsonValue>> {
    let members = members(&state).await.map_err(internal)?;
    Ok(Json(json!({"workers": members})))
}

/// The orchestrator and every worker whose membership has not expired, by id.
async fn members(state: &AppState) -> anyhow::Result<Vec<Member>> {
    let mut redis = state.redis.clone();
    let keys: Vec<String> = redis.keys(keys::MEMBER_PATTERN).await?;
    let records: Vec<Option<String>> = if keys.is_empty() {
        Vec::new()
    } else {
        redis.mget(&keys).await?
    };
    let mut members = vec![Member {
        id: 0,
        name: "orchestrator".to_string(),
        addr: state.mesh_addr.clone(),
        runtimes: Vec::new(),
        platform: String::new(),
    }];
    for record in records.into_iter().flatten() {
        members.push(serde_json::from_str(&record)?);
    }
    members.sort_by_key(|member| member.id);
    Ok(members)
}
