use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use bytes::Bytes;
use chrono::Utc;
use namu_proto::{
    Progress, RunCreateRequest, RunCreateResponse, RunStatusResponse, TaskCompleteRequest,
    TaskManifest, TaskStartRequest, WorkflowUploadRequest,
};
use redis::AsyncCommands;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use sha2::Digest;
use tar::Archive;
use uuid::Uuid;
use zstd::stream::read::Decoder;

use crate::{AppState, RunState, db, planner, redis_store, storage};

#[derive(Deserialize)]
pub struct EventsQuery {
    pub limit: Option<usize>,
}

pub async fn healthz() -> (StatusCode, Json<JsonValue>) {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

pub async fn upload_tasks(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<JsonValue>, StatusCode> {
    let mut artifact_bytes: Option<Bytes> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        if field.name() == Some("artifact") {
            artifact_bytes = Some(field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?);
        }
    }

    let bytes = artifact_bytes.ok_or(StatusCode::BAD_REQUEST)?;

    let (manifest, checksum) =
        read_manifest_from_tar(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;

    let artifact_path = storage::store_artifact(
        &state.artifacts_dir,
        &manifest.task_id,
        &manifest.version,
        &bytes,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    db::insert_task(&state.db, &manifest, &checksum)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    db::insert_task_artifact(
        &state.db,
        &manifest.task_id,
        &manifest.version,
        artifact_path.to_string_lossy().as_ref(),
        bytes.len() as i64,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "task_id": manifest.task_id,
        "version": manifest.version
    })))
}

pub async fn get_task_manifest(
    State(state): State<AppState>,
    Path((task_id, version)): Path<(String, String)>,
) -> Result<Json<TaskManifest>, StatusCode> {
    let manifest = db::get_task_manifest(&state.db, &task_id, &version)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(manifest))
}

pub async fn get_artifact(
    State(state): State<AppState>,
    Path((task_id, version)): Path<(String, String)>,
) -> Result<Bytes, StatusCode> {
    let bytes = storage::load_artifact(&state.artifacts_dir, &task_id, &version)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(bytes)
}

pub async fn upload_workflows(
    State(state): State<AppState>,
    Json(req): Json<WorkflowUploadRequest>,
) -> Result<Json<JsonValue>, StatusCode> {
    let _workflow: namu_core::ir::Workflow =
        serde_json::from_value(req.ir.clone()).map_err(|_| StatusCode::BAD_REQUEST)?;
    let task_versions = req.task_versions.clone();

    db::insert_workflow(&state.db, &req.id, &req.version, &req.ir, &task_versions)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "id": req.id,
        "version": req.version
    })))
}

pub async fn create_run(
    State(state): State<AppState>,
    Json(req): Json<RunCreateRequest>,
) -> Result<Json<RunCreateResponse>, StatusCode> {
    let run_id = db::create_run(&state.db, &req.workflow_id, &req.version)
        .await
        .map_err(|err| {
            tracing::error!("create_run: create_run failed: {err}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let (workflow, task_versions) = db::get_workflow(&state.db, &req.workflow_id, &req.version)
        .await
        .map_err(|err| {
            tracing::error!("create_run: get_workflow failed: {err}");
            StatusCode::NOT_FOUND
        })?;

    let run_state = RunState {
        workflow,
        task_versions,
        next_ctx_id: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(1)),
    };

    state.runs.write().await.insert(run_id, run_state);
    db::set_run_status(&state.db, run_id, "running")
        .await
        .map_err(|err| {
            tracing::error!("create_run: set_run_status failed: {err}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    db::create_context(&state.db, run_id, 0, None)
        .await
        .map_err(|err| {
            tracing::error!("create_run: create_context failed: {err}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let mut redis = state.redis.clone();
    redis_store::create_context(&mut redis, run_id, 0, None)
        .await
        .map_err(|err| {
            tracing::error!("create_run: redis create_context failed: {err}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    planner::drive_until_call(&state, run_id, 0, 0, None)
        .await
        .map_err(|err| {
            tracing::error!("create_run: drive_until_call failed: {err}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    update_run_status_if_complete(&state, run_id)
        .await
        .map_err(|err| {
            tracing::error!("create_run: update_run_status failed: {err}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(RunCreateResponse { run_id }))
}

pub async fn get_run_status(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<RunStatusResponse>, StatusCode> {
    let (done, total) = db::run_progress(&state.db, run_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let status = db::get_run_status(&state.db, run_id)
        .await
        .unwrap_or_else(|_| "unknown".to_string());
    Ok(Json(RunStatusResponse {
        status,
        progress: Progress { done, total },
    }))
}

pub async fn get_run_values(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<JsonValue>, StatusCode> {
    let key = format!("values:{run_id}:0");
    let mut conn = state.redis.clone();
    let values: BTreeMap<String, String> = conn
        .hgetall(key)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({"values": values})))
}

pub async fn run_events(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<JsonValue>, StatusCode> {
    let limit = query.limit.unwrap_or(100);
    let mut conn = state.redis.clone();
    let events = redis_store::read_events(&mut conn, run_id, limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({"events": events})))
}

pub async fn start_node(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
    Json(req): Json<TaskStartRequest>,
) -> Result<Json<JsonValue>, StatusCode> {
    let lease_expires_at = Utc::now() + chrono::Duration::milliseconds(req.lease_ms as i64);
    db::upsert_run_node(
        &state.db,
        run_id,
        req.op_id,
        req.ctx_id,
        "running",
        Some(lease_expires_at),
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({"status": "ok"})))
}

pub async fn submit_task(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
    Json(req): Json<TaskCompleteRequest>,
) -> Result<Json<JsonValue>, StatusCode> {
    if req.success {
        let output = req.output_json.ok_or(StatusCode::BAD_REQUEST)?;
        planner::apply_task_output(&state, run_id, req.op_id, req.ctx_id, output)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        db::update_run_node(&state.db, run_id, req.op_id, req.ctx_id, "succeeded", None)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    } else {
        db::update_run_node(
            &state.db,
            run_id,
            req.op_id,
            req.ctx_id,
            "failed",
            req.error.as_deref(),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        db::finish_context(&state.db, run_id, req.ctx_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    let mut redis = state.redis.clone();
    redis_store::add_event(
        &mut redis,
        run_id,
        &serde_json::json!({
            "event": if req.success {"completed"} else {"failed"},
            "op_id": req.op_id,
            "ctx_id": req.ctx_id
        }),
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    update_run_status_if_complete(&state, run_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({"status": "ok"})))
}

async fn update_run_status_if_complete(state: &AppState, run_id: Uuid) -> anyhow::Result<()> {
    let (done, total) = db::run_progress(&state.db, run_id).await?;
    let failed = db::count_nodes_by_status(&state.db, run_id, "failed").await?;
    let active = db::count_contexts_by_status(&state.db, run_id, "active").await?;

    if active == 0 {
        if total == 0 {
            db::set_run_status(&state.db, run_id, "succeeded").await?;
            return Ok(());
        }
        if done + failed >= total {
            let status = if failed > 0 {
                "partial_failed"
            } else {
                "succeeded"
            };
            db::set_run_status(&state.db, run_id, status).await?;
        }
    }
    Ok(())
}

pub async fn get_workers(State(state): State<AppState>) -> Result<Json<JsonValue>, StatusCode> {
    let workers = db::list_workers(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({"workers": workers})))
}

#[derive(Deserialize)]
pub struct RegisterWorkerRequest {
    pub worker_id: String,
    pub labels: JsonValue,
    pub resource_class: String,
    pub pool: String,
}

pub async fn register_worker(
    State(state): State<AppState>,
    Json(req): Json<RegisterWorkerRequest>,
) -> Result<Json<JsonValue>, StatusCode> {
    if !is_valid_pool(&req.pool) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut labels = req.labels;
    if labels.is_object() {
        labels["resource_class"] = JsonValue::String(req.resource_class);
        labels["pool"] = JsonValue::String(req.pool);
    }
    db::register_worker(&state.db, &req.worker_id, &labels, "ok")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({"status": "ok"})))
}

fn is_valid_pool(pool: &str) -> bool {
    matches!(pool, "trusted" | "restricted" | "wasm" | "gpu")
}

pub async fn expire_leases(state: &AppState) -> anyhow::Result<()> {
    let expired = db::expired_leases(&state.db).await?;
    for (run_id, op_id, ctx_id) in expired {
        db::update_run_node(
            &state.db,
            run_id,
            op_id as usize,
            ctx_id as usize,
            "failed",
            Some("lease expired"),
        )
        .await?;
        let mut redis = state.redis.clone();
        redis_store::add_event(
            &mut redis,
            run_id,
            &serde_json::json!({"event": "lease_expired", "op_id": op_id, "ctx_id": ctx_id}),
        )
        .await?;
    }
    Ok(())
}

fn read_manifest_from_tar(bytes: &Bytes) -> anyhow::Result<(TaskManifest, String)> {
    let decoder = Decoder::new(Cursor::new(bytes))?;
    let mut archive = Archive::new(decoder);
    let mut manifest: Option<TaskManifest> = None;
    let mut checksum = None;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path
            .file_name()
            .map(|n| n == "manifest.json")
            .unwrap_or(false)
        {
            let mut contents = String::new();
            entry.read_to_string(&mut contents)?;
            let parsed: TaskManifest = serde_json::from_str(&contents)?;
            manifest = Some(parsed);
        } else if path.file_name().is_some() {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            let digest = sha2::Sha256::digest(&buf);
            checksum = Some(format!("sha256:{:x}", digest));
        }
    }

    let manifest = manifest.ok_or_else(|| anyhow::anyhow!("missing manifest.json"))?;
    let checksum = checksum.ok_or_else(|| anyhow::anyhow!("missing library for checksum"))?;
    Ok((manifest, checksum))
}
