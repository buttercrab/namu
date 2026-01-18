use std::collections::HashMap;

use namu_core::ir::Workflow;
use namu_proto::TaskManifest;
use serde_json::Value as JsonValue;
use sqlx_core::row::Row;
use sqlx_postgres::{PgPool, Postgres};
use uuid::Uuid;

const DDL: &str = r#"
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS tasks (
  id TEXT NOT NULL,
  version TEXT NOT NULL,
  manifest_json JSONB NOT NULL,
  checksum TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (id, version)
);

CREATE TABLE IF NOT EXISTS task_artifacts (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  task_id TEXT NOT NULL,
  task_version TEXT NOT NULL,
  uri TEXT NOT NULL,
  size_bytes BIGINT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS workflows (
  id TEXT NOT NULL,
  version TEXT NOT NULL,
  ir_json JSONB NOT NULL,
  task_versions JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (id, version)
);

CREATE TABLE IF NOT EXISTS runs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  workflow_id TEXT NOT NULL,
  workflow_version TEXT NOT NULL,
  status TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS contexts (
  id INT NOT NULL,
  run_id UUID NOT NULL,
  parent_ctx_id INT,
  status TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (run_id, id)
);

CREATE TABLE IF NOT EXISTS run_nodes (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  run_id UUID NOT NULL,
  op_id INT NOT NULL,
  ctx_id INT NOT NULL,
  status TEXT NOT NULL,
  retries INT NOT NULL DEFAULT 0,
  last_error TEXT,
  lease_expires_at TIMESTAMPTZ,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (run_id, op_id, ctx_id)
);

CREATE TABLE IF NOT EXISTS workers (
  id TEXT PRIMARY KEY,
  labels_json JSONB NOT NULL,
  status TEXT NOT NULL,
  last_heartbeat TIMESTAMPTZ NOT NULL
);
"#;

pub async fn init_db(pool: &PgPool) -> anyhow::Result<()> {
    for statement in DDL.split(';') {
        let stmt = statement.trim();
        if stmt.is_empty() {
            continue;
        }
        sqlx_core::query::query::<Postgres>(stmt)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn insert_task(
    pool: &PgPool,
    manifest: &TaskManifest,
    checksum: &str,
) -> anyhow::Result<()> {
    let manifest_json = serde_json::to_value(manifest)?;
    sqlx_core::query::query::<Postgres>(
        r#"
        INSERT INTO tasks (id, version, manifest_json, checksum)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (id, version)
        DO UPDATE SET manifest_json = EXCLUDED.manifest_json, checksum = EXCLUDED.checksum
        "#,
    )
    .bind(&manifest.task_id)
    .bind(&manifest.version)
    .bind(manifest_json)
    .bind(checksum)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_task_artifact(
    pool: &PgPool,
    task_id: &str,
    task_version: &str,
    uri: &str,
    size_bytes: i64,
) -> anyhow::Result<()> {
    sqlx_core::query::query::<Postgres>(
        r#"
        INSERT INTO task_artifacts (task_id, task_version, uri, size_bytes)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(task_id)
    .bind(task_version)
    .bind(uri)
    .bind(size_bytes)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_task_manifest(
    pool: &PgPool,
    task_id: &str,
    version: &str,
) -> anyhow::Result<TaskManifest> {
    let row = sqlx_core::query::query::<Postgres>(
        "SELECT manifest_json FROM tasks WHERE id = $1 AND version = $2",
    )
    .bind(task_id)
    .bind(version)
    .fetch_one(pool)
    .await?;
    let manifest_json: JsonValue = row.try_get("manifest_json")?;
    let manifest: TaskManifest = serde_json::from_value(manifest_json)?;
    Ok(manifest)
}

pub async fn insert_workflow(
    pool: &PgPool,
    id: &str,
    version: &str,
    ir: &JsonValue,
    task_versions: &HashMap<String, String>,
) -> anyhow::Result<()> {
    let task_versions_json = serde_json::to_value(task_versions)?;
    sqlx_core::query::query::<Postgres>(
        r#"
        INSERT INTO workflows (id, version, ir_json, task_versions)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (id, version)
        DO UPDATE SET ir_json = EXCLUDED.ir_json, task_versions = EXCLUDED.task_versions
        "#,
    )
    .bind(id)
    .bind(version)
    .bind(ir)
    .bind(task_versions_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_workflow(
    pool: &PgPool,
    id: &str,
    version: &str,
) -> anyhow::Result<(Workflow, HashMap<String, String>)> {
    let row = sqlx_core::query::query::<Postgres>(
        "SELECT ir_json, task_versions FROM workflows WHERE id = $1 AND version = $2",
    )
    .bind(id)
    .bind(version)
    .fetch_one(pool)
    .await?;
    let ir_json: JsonValue = row.try_get("ir_json")?;
    let task_versions_json: JsonValue = row.try_get("task_versions")?;
    let workflow: Workflow = serde_json::from_value(ir_json)?;
    let task_versions: HashMap<String, String> = serde_json::from_value(task_versions_json)?;
    Ok((workflow, task_versions))
}

pub async fn create_run(
    pool: &PgPool,
    workflow_id: &str,
    workflow_version: &str,
) -> anyhow::Result<Uuid> {
    let row = sqlx_core::query::query::<Postgres>(
        r#"
        INSERT INTO runs (workflow_id, workflow_version, status)
        VALUES ($1, $2, 'queued')
        RETURNING id
        "#,
    )
    .bind(workflow_id)
    .bind(workflow_version)
    .fetch_one(pool)
    .await?;
    let id: Uuid = row.try_get("id")?;
    Ok(id)
}

pub async fn set_run_status(pool: &PgPool, run_id: Uuid, status: &str) -> anyhow::Result<()> {
    sqlx_core::query::query::<Postgres>(
        "UPDATE runs SET status = $1, updated_at = now() WHERE id = $2",
    )
    .bind(status)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert_run_node(
    pool: &PgPool,
    run_id: Uuid,
    op_id: usize,
    ctx_id: usize,
    status: &str,
    lease_expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> anyhow::Result<()> {
    sqlx_core::query::query::<Postgres>(
        r#"
        INSERT INTO run_nodes (run_id, op_id, ctx_id, status, lease_expires_at)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (run_id, op_id, ctx_id)
        DO UPDATE SET status = EXCLUDED.status, lease_expires_at = EXCLUDED.lease_expires_at, updated_at = now()
        "#,
    )
    .bind(run_id)
    .bind(op_id as i32)
    .bind(ctx_id as i32)
    .bind(status)
    .bind(lease_expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_run_node(
    pool: &PgPool,
    run_id: Uuid,
    op_id: usize,
    ctx_id: usize,
    status: &str,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx_core::query::query::<Postgres>(
        r#"
        UPDATE run_nodes
        SET status = $1, last_error = $2, updated_at = now()
        WHERE run_id = $3 AND op_id = $4 AND ctx_id = $5
        "#,
    )
    .bind(status)
    .bind(error)
    .bind(run_id)
    .bind(op_id as i32)
    .bind(ctx_id as i32)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn create_context(
    pool: &PgPool,
    run_id: Uuid,
    ctx_id: usize,
    parent_ctx_id: Option<usize>,
) -> anyhow::Result<()> {
    sqlx_core::query::query::<Postgres>(
        r#"
        INSERT INTO contexts (run_id, id, parent_ctx_id, status)
        VALUES ($1, $2, $3, 'active')
        ON CONFLICT (run_id, id) DO NOTHING
        "#,
    )
    .bind(run_id)
    .bind(ctx_id as i32)
    .bind(parent_ctx_id.map(|id| id as i32))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn finish_context(pool: &PgPool, run_id: Uuid, ctx_id: usize) -> anyhow::Result<()> {
    sqlx_core::query::query::<Postgres>(
        "UPDATE contexts SET status = 'done' WHERE run_id = $1 AND id = $2",
    )
    .bind(run_id)
    .bind(ctx_id as i32)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn run_progress(pool: &PgPool, run_id: Uuid) -> anyhow::Result<(usize, usize)> {
    let row = sqlx_core::query::query::<Postgres>(
        "SELECT COUNT(*) FILTER (WHERE status = 'succeeded') AS done, COUNT(*) AS total FROM run_nodes WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    let done: i64 = row.try_get("done")?;
    let total: i64 = row.try_get("total")?;
    Ok((done as usize, total as usize))
}

pub async fn count_nodes_by_status(
    pool: &PgPool,
    run_id: Uuid,
    status: &str,
) -> anyhow::Result<usize> {
    let row = sqlx_core::query::query::<Postgres>(
        "SELECT COUNT(*) AS count FROM run_nodes WHERE run_id = $1 AND status = $2",
    )
    .bind(run_id)
    .bind(status)
    .fetch_one(pool)
    .await?;
    let count: i64 = row.try_get("count")?;
    Ok(count as usize)
}

pub async fn count_contexts_by_status(
    pool: &PgPool,
    run_id: Uuid,
    status: &str,
) -> anyhow::Result<usize> {
    let row = sqlx_core::query::query::<Postgres>(
        "SELECT COUNT(*) AS count FROM contexts WHERE run_id = $1 AND status = $2",
    )
    .bind(run_id)
    .bind(status)
    .fetch_one(pool)
    .await?;
    let count: i64 = row.try_get("count")?;
    Ok(count as usize)
}

pub async fn get_run_status(pool: &PgPool, run_id: Uuid) -> anyhow::Result<String> {
    let row = sqlx_core::query::query::<Postgres>("SELECT status FROM runs WHERE id = $1")
        .bind(run_id)
        .fetch_one(pool)
        .await?;
    let status: String = row.try_get("status")?;
    Ok(status)
}

pub async fn list_workers(pool: &PgPool) -> anyhow::Result<Vec<JsonValue>> {
    let rows = sqlx_core::query::query::<Postgres>(
        "SELECT id, labels_json, status, last_heartbeat FROM workers",
    )
    .fetch_all(pool)
    .await?;
    let mut workers = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id")?;
        let labels: JsonValue = row.try_get("labels_json")?;
        let status: String = row.try_get("status")?;
        let heartbeat: chrono::DateTime<chrono::Utc> = row.try_get("last_heartbeat")?;
        workers.push(serde_json::json!({
            "id": id,
            "labels": labels,
            "status": status,
            "last_heartbeat": heartbeat,
        }));
    }
    Ok(workers)
}

pub async fn register_worker(
    pool: &PgPool,
    worker_id: &str,
    labels: &JsonValue,
    status: &str,
) -> anyhow::Result<()> {
    sqlx_core::query::query::<Postgres>(
        r#"
        INSERT INTO workers (id, labels_json, status, last_heartbeat)
        VALUES ($1, $2, $3, now())
        ON CONFLICT (id) DO UPDATE SET labels_json = EXCLUDED.labels_json, status = EXCLUDED.status, last_heartbeat = now()
        "#,
    )
    .bind(worker_id)
    .bind(labels)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn expired_leases(pool: &PgPool) -> anyhow::Result<Vec<(Uuid, i32, i32)>> {
    let rows = sqlx_core::query::query::<Postgres>(
        "SELECT run_id, op_id, ctx_id FROM run_nodes WHERE status = 'running' AND lease_expires_at IS NOT NULL AND lease_expires_at < now()",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let run_id: Uuid = row.try_get("run_id")?;
        let op_id: i32 = row.try_get("op_id")?;
        let ctx_id: i32 = row.try_get("ctx_id")?;
        out.push((run_id, op_id, ctx_id));
    }
    Ok(out)
}
