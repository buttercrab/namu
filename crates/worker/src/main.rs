use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct RegisterWorkerRequest {
    worker_id: String,
    address: String,
    port: u16,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let master_url = env::var("MASTER_URL").unwrap_or_else(|_| "ws://localhost:8080".to_string());
    let worker_id = env::var("WORKER_ID").unwrap_or_else(|_| Uuid::new_v4().to_string());
    let worker_address = env::var("WORKER_ADDRESS").unwrap_or_else(|_| "127.0.0.1".to_string());
    let worker_port: u16 = env::var("WORKER_PORT")
        .unwrap_or_else(|_| "9000".to_string())
        .parse()
        .expect("Invalid WORKER_PORT");

    info!("Worker starting with ID: {}", worker_id);
    info!("Master WebSocket URL: {}", master_url);
    info!("Worker address: {}:{}", worker_address, worker_port);

    // Connect to master via WebSocket
    let ws_url = format!("{}/workers/ws", master_url);

    loop {
        match connect_to_master(&ws_url, &worker_id, &worker_address, worker_port).await {
            Ok(_) => {
                info!("WebSocket connection to master closed, attempting to reconnect...");
            }
            Err(e) => {
                error!("Failed to connect to master: {}", e);
            }
        }

        // Wait before reconnecting
        time::sleep(Duration::from_secs(5)).await;
    }
}

async fn connect_to_master(
    ws_url: &str,
    worker_id: &str,
    address: &str,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Connecting to master at: {}", ws_url);

    let (ws_stream, _) = connect_async(ws_url).await?;
    info!("WebSocket connection established");

    let (mut write, mut read) = ws_stream.split();

    // Send registration message
    let register_payload = RegisterWorkerRequest {
        worker_id: worker_id.to_string(),
        address: address.to_string(),
        port,
    };

    let register_msg = serde_json::to_string(&register_payload)?;
    write.send(Message::Text(register_msg)).await?;
    info!("Registration message sent");

    // Handle incoming messages
    while let Some(msg) = read.next().await {
        match msg? {
            Message::Text(text) => {
                info!("Received message from master: {}", text);
                // Handle registration confirmation or other text messages
            }
            Message::Ping(payload) => {
                info!("Received ping from master, sending pong");
                write.send(Message::Pong(payload)).await?;
            }
            Message::Pong(_) => {
                info!("Received pong from master");
            }
            Message::Close(_) => {
                info!("Master closed the connection");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
