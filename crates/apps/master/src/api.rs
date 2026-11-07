//! The HTTP API: each route reads its request and hands it to [`Control`].

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, MatchedPath, Multipart, Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use namu_proto::{
    Membership, Page, PageQuery, Published, RunCreateRequest, RunCreateResponse, RunStatusResponse,
    TaskManifest, WorkerRegisterRequest, WorkerRegisterResponse, WorkersResponse,
    WorkflowUploadRequest,
};
use serde_json::json;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::auth::{self, Require, Role, Tokens};
use crate::control::Control;
use crate::error::ApiError;

type Api<T> = Result<Json<T>, ApiError>;

/// Task artifacts carry compiled code and, for some, a model: allow them to be large.
const MAX_BODY: usize = 256 * 1024 * 1024;

/// The API, each route open to the roles that need it (all of them, if `tokens` is `None`).
pub fn router(control: Arc<Control>, tokens: Option<Arc<Tokens>>) -> Router {
    let only = |roles: &'static [Role]| {
        let require = Require {
            tokens: tokens.clone(),
            roles,
        };
        middleware::from_fn_with_state(require, auth::require)
    };
    // Publishing, running and reading runs.
    let clients = Router::new()
        .route("/tasks", post(publish_task))
        .route("/workflows", post(publish_workflow))
        .route("/runs", post(start_run))
        .route("/runs/{run_id}", get(run_status))
        .route("/runs/{run_id}/results", get(results))
        .route("/runs/{run_id}/failures", get(failures))
        .route("/workers", get(workers))
        .route_layer(only(&[Role::Client]));
    // Joining the mesh and staying in it.
    let workers = Router::new()
        .route("/workers/register", post(register))
        .route("/workers/{worker_id}/heartbeat", post(heartbeat))
        .route("/workers/{worker_id}/drain", post(drain))
        .route("/workers/{worker_id}/leave", post(leave))
        .route_layer(only(&[Role::Worker]));
    // What tasks are: workers load them, clients look them up.
    let both = Router::new()
        .route("/tasks/{task_id}/{version}", get(task))
        .route("/tasks/{task_id}/{version}/artifact", get(artifact))
        .route_layer(only(&[Role::Client, Role::Worker]));
    Router::new()
        .route("/healthz", get(healthz))
        .merge(clients)
        .merge(workers)
        .merge(both)
        .layer(middleware::from_fn(measure))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(TraceLayer::new_for_http())
        .with_state(control)
}

/// Counts and times each request, by route (its pattern: ids would make a series each) and status.
async fn measure(route: Option<MatchedPath>, request: Request, next: Next) -> Response {
    let route = route.map_or_else(|| "unmatched".to_string(), |r| r.as_str().to_string());
    let started = std::time::Instant::now();
    let response = next.run(request).await;
    let status = response.status().as_u16().to_string();
    metrics::counter!("namu_http_requests_total", "route" => route.clone(), "status" => status)
        .increment(1);
    metrics::histogram!("namu_http_request_seconds", "route" => route)
        .record(started.elapsed().as_secs_f64());
    response
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

/// `?after=N&limit=M`: the results from the `N`th on, in the order they arrived.
async fn results(
    State(control): State<Arc<Control>>,
    Path(run): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Api<Page<serde_json::Value>> {
    Ok(Json(control.results(run, query).await?))
}

async fn failures(
    State(control): State<Arc<Control>>,
    Path(run): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Api<Page<String>> {
    Ok(Json(control.failures(run, query).await?))
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
