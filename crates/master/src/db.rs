use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Row, SqlitePool};
use tracing::info;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Worker {
    pub id: String,
    pub address: String,
    pub port: u16,
    pub last_heartbeat: i64,
    pub status: String,
}

pub async fn init_db() -> Result<SqlitePool, sqlx::Error> {
    let db_path = "database.db";

    // Check if database file exists, create directory if needed
    if let Some(parent) = Path::new(db_path).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(sqlx::Error::Io)?;
    }

    // Check if database file exists
    if !Path::new(db_path).exists() {
        info!(
            "Database file not found, creating new database at: {}",
            db_path
        );
        // Touch the file to create it
        tokio::fs::File::create(db_path)
            .await
            .map_err(sqlx::Error::Io)?;
    } else {
        info!("Using existing database file: {}", db_path);
    }

    // Connect to database
    let pool = SqlitePool::connect(&format!("sqlite://{}", db_path)).await?;

    // Create workers table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS workers (
            id TEXT PRIMARY KEY,
            address TEXT NOT NULL,
            port INTEGER NOT NULL,
            last_heartbeat INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'active'
        )
        "#,
    )
    .execute(&pool)
    .await?;

    info!(
        "SQLite database initialized with workers table at: {}",
        db_path
    );
    Ok(pool)
}

pub async fn health_check_db(pool: &SqlitePool) -> Result<i32, sqlx::Error> {
    let row = sqlx::query("SELECT 1 as result").fetch_one(pool).await?;
    let result: i32 = row.get("result");
    Ok(result)
}

pub async fn register_worker(
    pool: &SqlitePool,
    worker_id: &str,
    address: &str,
    port: u16,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().timestamp();

    sqlx::query(
        r#"
        INSERT OR REPLACE INTO workers (id, address, port, last_heartbeat, status)
        VALUES (?, ?, ?, ?, 'active')
        "#,
    )
    .bind(worker_id)
    .bind(address)
    .bind(port)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn update_worker_heartbeat(
    pool: &SqlitePool,
    worker_id: &str,
) -> Result<bool, sqlx::Error> {
    let now = chrono::Utc::now().timestamp();

    let result = sqlx::query(
        r#"
        UPDATE workers 
        SET last_heartbeat = ?, status = 'active'
        WHERE id = ?
        "#,
    )
    .bind(now)
    .bind(worker_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn get_all_workers(pool: &SqlitePool) -> Result<Vec<Worker>, sqlx::Error> {
    let workers = sqlx::query_as::<_, Worker>("SELECT * FROM workers")
        .fetch_all(pool)
        .await?;

    Ok(workers)
}

pub async fn cleanup_inactive_workers(
    pool: &SqlitePool,
    timeout_seconds: i64,
) -> Result<u64, sqlx::Error> {
    let cutoff = chrono::Utc::now().timestamp() - timeout_seconds;

    let result = sqlx::query(
        r#"
        UPDATE workers 
        SET status = 'inactive'
        WHERE last_heartbeat < ? AND status = 'active'
        "#,
    )
    .bind(cutoff)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub async fn remove_worker(pool: &SqlitePool, worker_id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM workers WHERE id = ?")
        .bind(worker_id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}
