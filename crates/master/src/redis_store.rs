use namu_proto::QueueMessage;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde_json::Value as JsonValue;
use uuid::Uuid;

fn values_key(run_id: Uuid, ctx_id: usize) -> String {
    format!("values:{run_id}:{ctx_id}")
}

pub async fn set_value(
    conn: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
    value_id: usize,
    value: &JsonValue,
) -> anyhow::Result<()> {
    let key = values_key(run_id, ctx_id);
    let field = value_id.to_string();
    let payload = serde_json::to_string(value)?;
    let _: () = conn.hset(key, field, payload).await?;
    Ok(())
}

pub async fn get_value(
    conn: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
    value_id: usize,
) -> anyhow::Result<Option<JsonValue>> {
    let key = values_key(run_id, ctx_id);
    let field = value_id.to_string();
    let payload: Option<String> = conn.hget(key, field).await?;
    match payload {
        Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        None => Ok(None),
    }
}

pub async fn clone_context(
    conn: &mut ConnectionManager,
    run_id: Uuid,
    src_ctx_id: usize,
    dst_ctx_id: usize,
) -> anyhow::Result<()> {
    let src_key = values_key(run_id, src_ctx_id);
    let dst_key = values_key(run_id, dst_ctx_id);
    let entries: Vec<(String, String)> = conn.hgetall(src_key).await?;
    if entries.is_empty() {
        return Ok(());
    }
    let _: () = conn.hset_multiple(dst_key, &entries).await?;
    Ok(())
}

pub async fn queue_task(
    conn: &mut ConnectionManager,
    resource_class: &str,
    msg: &QueueMessage,
) -> anyhow::Result<()> {
    let key = format!("queue:{resource_class}");
    let payload = serde_json::to_string(msg)?;
    let _: String = redis::cmd("XADD")
        .arg(&key)
        .arg("*")
        .arg("payload")
        .arg(payload)
        .query_async(conn)
        .await?;
    Ok(())
}

pub async fn add_event(
    conn: &mut ConnectionManager,
    run_id: Uuid,
    payload: &JsonValue,
) -> anyhow::Result<()> {
    let key = format!("events:{run_id}");
    let payload = serde_json::to_string(payload)?;
    let _: String = redis::cmd("XADD")
        .arg(&key)
        .arg("*")
        .arg("payload")
        .arg(payload)
        .query_async(conn)
        .await?;
    Ok(())
}

pub async fn read_events(
    conn: &mut ConnectionManager,
    run_id: Uuid,
    limit: usize,
) -> anyhow::Result<Vec<JsonValue>> {
    let key = format!("events:{run_id}");
    let reply: redis::Value = redis::cmd("XREVRANGE")
        .arg(&key)
        .arg("+")
        .arg("-")
        .arg("COUNT")
        .arg(limit as i64)
        .query_async(conn)
        .await?;
    parse_stream_payloads(reply)
}

fn parse_stream_payloads(value: redis::Value) -> anyhow::Result<Vec<JsonValue>> {
    let mut out = Vec::new();
    let entries = match value {
        redis::Value::Array(entries) => entries,
        _ => return Ok(out),
    };

    for entry in entries {
        let redis::Value::Array(parts) = entry else {
            continue;
        };
        if parts.len() != 2 {
            continue;
        }
        let redis::Value::Array(kv) = &parts[1] else {
            continue;
        };
        let mut i = 0;
        while i + 1 < kv.len() {
            let key = &kv[i];
            let val = &kv[i + 1];
            let field = redis::from_redis_value::<String>(key)?;
            if field == "payload" {
                let payload = redis::from_redis_value::<String>(val)?;
                out.push(serde_json::from_str(&payload)?);
            }
            i += 2;
        }
    }
    Ok(out)
}
