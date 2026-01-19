use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::WebSocket;
use axum::extract::{State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{Json, Response};
use futures_util::{SinkExt, StreamExt};
use hashbrown::HashMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio::sync::{Mutex, RwLock};
use tokio::time;
use tracing::{error, info};

use crate::db;

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterWorkerRequest {
    pub worker_id: String,
    pub address: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct WorkerConnection {
    pub _worker_id: String,
    pub _address: String,
    pub _port: u16,
    pub sender: Arc<Mutex<futures_util::stream::SplitSink<WebSocket, axum::extract::ws::Message>>>,
}

pub type WorkerConnections = Arc<RwLock<HashMap<String, WorkerConnection>>>;

pub async fn handle_worker_websocket(
    ws: WebSocketUpgrade,
    State((pool, connections)): State<(SqlitePool, WorkerConnections)>,
) -> Response {
    ws.on_upgrade(|socket| websocket_handler(socket, pool, connections))
}

async fn websocket_handler(socket: WebSocket, pool: SqlitePool, connections: WorkerConnections) {
    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(Mutex::new(sender));

    let mut worker_id = None;

    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(axum::extract::ws::Message::Text(text)) => {
                match serde_json::from_str::<RegisterWorkerRequest>(&text) {
                    Ok(register_req) => {
                        info!(
                            "Worker connecting via WebSocket: {}",
                            register_req.worker_id
                        );

                        // Register in database
                        if let Err(e) = db::register_worker(
                            &pool,
                            &register_req.worker_id,
                            &register_req.address,
                            register_req.port,
                        )
                        .await
                        {
                            error!("Failed to register worker in database: {}", e);
                            break;
                        }

                        // Store connection
                        let connection = WorkerConnection {
                            _worker_id: register_req.worker_id.clone(),
                            _address: register_req.address,
                            _port: register_req.port,
                            sender: sender.clone(),
                        };

                        connections
                            .write()
                            .await
                            .insert(register_req.worker_id.clone(), connection);
                        worker_id = Some(register_req.worker_id.clone());

                        // Send confirmation
                        let response =
                            json!({"status": "registered", "worker_id": register_req.worker_id});
                        if let Ok(msg) = serde_json::to_string(&response)
                            && let Err(e) = sender
                                .lock()
                                .await
                                .send(axum::extract::ws::Message::Text(msg.into()))
                                .await
                        {
                            error!("Failed to send registration confirmation: {}", e);
                            break;
                        }
                    }
                    Err(_) => {
                        // Handle other message types if needed
                    }
                }
            }
            Ok(axum::extract::ws::Message::Pong(_)) => {
                // Update heartbeat in database
                if let Some(ref wid) = worker_id
                    && let Err(e) = db::update_worker_heartbeat(&pool, wid).await
                {
                    error!("Failed to update heartbeat for worker {}: {}", wid, e);
                }
            }
            Ok(axum::extract::ws::Message::Close(_)) => {
                info!("Worker WebSocket connection closed");
                break;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    // Clean up connection when socket closes
    if let Some(wid) = worker_id {
        connections.write().await.remove(&wid);
        info!("Removed worker connection: {}", wid);
    }
}

pub async fn list_workers(
    State((pool, _)): State<(SqlitePool, WorkerConnections)>,
) -> (StatusCode, Json<Value>) {
    match db::get_all_workers(&pool).await {
        Ok(workers) => (StatusCode::OK, Json(json!({ "workers": workers }))),
        Err(e) => {
            error!("Failed to list workers: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"status": "error", "message": "Failed to list workers"})),
            )
        }
    }
}

pub async fn start_heartbeat_monitor(pool: SqlitePool, connections: WorkerConnections) {
    let mut interval = time::interval(Duration::from_secs(30));

    loop {
        interval.tick().await;

        // Send ping to all connected workers
        let connections_read = connections.read().await;
        let worker_ids: Vec<String> = connections_read.keys().cloned().collect();
        drop(connections_read);

        for worker_id in worker_ids {
            let connections_read = connections.read().await;
            if let Some(connection) = connections_read.get(&worker_id) {
                let sender = connection.sender.clone();
                let wid = worker_id.clone();
                drop(connections_read);

                // Send ping
                match sender
                    .lock()
                    .await
                    .send(axum::extract::ws::Message::Ping(vec![].into()))
                    .await
                {
                    Ok(_) => {
                        info!("Sent ping to worker: {}", wid);
                    }
                    Err(e) => {
                        error!("Failed to send ping to worker {}: {}", wid, e);
                        // Remove failed connection
                        connections.write().await.remove(&wid);

                        // Remove worker from database
                        if let Err(e) = db::remove_worker(&pool, &wid).await {
                            error!("Failed to remove worker {} from database: {}", wid, e);
                        } else {
                            info!("Removed failed worker {} from database", wid);
                        }
                    }
                }
            }
        }

        // Clean up inactive workers from database
        match db::cleanup_inactive_workers(&pool, 60).await {
            Ok(count) => {
                if count > 0 {
                    info!("Marked {} workers as inactive", count);
                }
            }
            Err(e) => {
                error!("Failed to cleanup inactive workers: {}", e);
            }
        }
    }
}
