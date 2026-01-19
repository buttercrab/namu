use namu_proto::QueueMessage;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde_json::Value as JsonValue;
use uuid::Uuid;

const MAX_PARENT_HOPS: usize = 10_000;

fn values_key(run_id: Uuid, ctx_id: usize) -> String {
    format!("values:{run_id}:{ctx_id}")
}

fn context_key(run_id: Uuid, ctx_id: usize) -> String {
    format!("context:{run_id}:{ctx_id}")
}

pub async fn create_context(
    conn: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
    parent_ctx_id: Option<usize>,
) -> anyhow::Result<()> {
    let key = context_key(run_id, ctx_id);
    let parent = parent_ctx_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-1".to_string());
    let _: () = conn.hset(key, "parent", parent).await?;
    Ok(())
}

async fn get_parent(
    conn: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
) -> anyhow::Result<Option<usize>> {
    let key = context_key(run_id, ctx_id);
    let raw: Option<String> = conn.hget(key, "parent").await?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_empty() || raw == "-1" {
        return Ok(None);
    }
    let parent = raw
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("invalid parent ctx id {raw}"))?;
    Ok(Some(parent))
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
    let payload = get_value_raw(conn, run_id, ctx_id, value_id).await?;
    match payload {
        Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        None => Ok(None),
    }
}

pub async fn get_value_raw(
    conn: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
    value_id: usize,
) -> anyhow::Result<Option<String>> {
    let field = value_id.to_string();
    let mut current_ctx = Some(ctx_id);
    let mut hops = 0usize;

    while let Some(ctx) = current_ctx {
        let key = values_key(run_id, ctx);
        if let Some(payload) = conn.hget::<_, _, Option<String>>(key, &field).await? {
            return Ok(Some(payload));
        }
        current_ctx = get_parent(conn, run_id, ctx).await?;
        hops = hops.saturating_add(1);
        if hops > MAX_PARENT_HOPS {
            return Err(anyhow::anyhow!(
                "context parent chain exceeded {MAX_PARENT_HOPS} hops"
            ));
        }
    }

    Ok(None)
}

pub async fn queue_task(
    conn: &mut ConnectionManager,
    pool: &str,
    resource_class: &str,
    msg: &QueueMessage,
) -> anyhow::Result<()> {
    let key = format!("queue:{pool}:{resource_class}");
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
