//! The HTTP API: each route reads its request and hands it to [`Control`].

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use namu_proto::{
    Membership, Published, RunCreateRequest, RunCreateResponse, RunStatusResponse, TaskManifest,
    WorkerRegisterRequest, WorkerRegisterResponse, WorkersResponse, WorkflowUploadRequest,
};
use serde_json::json;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::control::Control;
use crate::error::ApiError;

type Api<T> = Result<Json<T>, ApiError>;

/// Task artifacts carry compiled code and, for some, a model: allow them to be large.
const MAX_BODY: usize = 256 * 1024 * 1024;

pub fn router(control: Arc<Control>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/tasks", post(publish_task))
        .route("/tasks/{task_id}/{version}", get(task))
        .route("/tasks/{task_id}/{version}/artifact", get(artifact))
        .route("/workflows", post(publish_workflow))
        .route("/runs", post(start_run))
        .route("/runs/{run_id}", get(run_status))
        .route("/workers", get(workers))
        .route("/workers/register", post(register))
        .route("/workers/{worker_id}/heartbeat", post(heartbeat))
        .route("/workers/{worker_id}/drain", post(drain))
        .route("/workers/{worker_id}/leave", post(leave))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(TraceLayer::new_for_http())
        .with_state(control)
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}

/// Publishes a task version: a multipart form whose `artifact` field is its package.
async fn publish_task(State(control): State<Arc<Control>>, mut form: Multipart) -> Api<Published> {
    let mut artifact = None;
    while let Some(field) = form.next_field().await.map_err(ApiError::bad_request)? {
        if field.name() == Some("artifact") {
            artifact = Some(field.bytes().await.map_err(ApiError::bad_request)?);
        }
    }
    let artifact = artifact.ok_or_else(|| ApiError::bad_request("no `artifact` field"))?;
    Ok(Json(control.publish_task(artifact).await?))
}

async fn task(
    State(control): State<Arc<Control>>,
    Path((id, version)): Path<(String, String)>,
) -> Api<TaskManifest> {
    Ok(Json(control.task(&id, &version).await?))
}

async fn artifact(
    State(control): State<Arc<Control>>,
    Path((id, version)): Path<(String, String)>,
) -> Result<Bytes, ApiError> {
    control.artifact(&id, &version).await
}

async fn publish_workflow(
    State(control): State<Arc<Control>>,
    Json(upload): Json<WorkflowUploadRequest>,
) -> Api<Published> {
    Ok(Json(control.publish_workflow(upload).await?))
}

async fn start_run(
    State(control): State<Arc<Control>>,
    Json(request): Json<RunCreateRequest>,
) -> Api<RunCreateResponse> {
    let run_id = control.start_run(request).await?;
    Ok(Json(RunCreateResponse { run_id }))
}

async fn run_status(
    State(control): State<Arc<Control>>,
    Path(run): Path<Uuid>,
) -> Api<RunStatusResponse> {
    Ok(Json(control.run_status(run).await?))
}

async fn workers(State(control): State<Arc<Control>>) -> Api<WorkersResponse> {
    let membership = control.membership().await?;
    Ok(Json(WorkersResponse {
        workers: membership.members,
    }))
}

async fn register(
    State(control): State<Arc<Control>>,
    Json(request): Json<WorkerRegisterRequest>,
) -> Api<WorkerRegisterResponse> {
    Ok(Json(control.register(request).await?))
}

async fn heartbeat(
    State(control): State<Arc<Control>>,
    Path(worker): Path<u64>,
) -> Api<Membership> {
    Ok(Json(control.heartbeat(worker).await?))
}

async fn drain(State(control): State<Arc<Control>>, Path(worker): Path<u64>) -> Api<Membership> {
    Ok(Json(control.drain(worker).await?))
}

async fn leave(
    State(control): State<Arc<Control>>,
    Path(worker): Path<u64>,
) -> Result<StatusCode, ApiError> {
    control.leave(worker).await?;
    Ok(StatusCode::NO_CONTENT)
}
