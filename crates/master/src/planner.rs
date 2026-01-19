use namu_core::ir::{Call, Next, Operation};
use namu_proto::{QueueMessage, TaskKind, TaskRuntime, TaskTrust, ValueRef};
use redis::aio::ConnectionManager;
use serde_json::Value as JsonValue;
use sha2::Digest;
use uuid::Uuid;

use crate::{AppState, RunState, db, object_store, redis_store};

pub async fn drive_until_call(
    state: &AppState,
    run_id: Uuid,
    ctx_id: usize,
    start_op: usize,
    pred_op: Option<usize>,
) -> anyhow::Result<()> {
    let run_state = get_run_state(state, run_id).await?;
    let workflow = run_state.workflow.clone();
    let mut op_id = start_op;
    let mut pred = pred_op;

    let mut redis = state.redis.clone();

    loop {
        let operation = &workflow.operations[op_id];
        apply_literals(&mut redis, run_id, ctx_id, operation).await?;
        apply_phis(&mut redis, run_id, ctx_id, operation, pred).await?;

        if let Some(call) = &operation.call {
            enqueue_call(state, &run_state, run_id, ctx_id, op_id, call).await?;
            return Ok(());
        }

        match next_op_id(&mut redis, run_id, ctx_id, &operation.next).await? {
            Some(next) => {
                pred = Some(op_id);
                op_id = next;
            }
            None => {
                db::finish_context(&state.db, run_id, ctx_id).await?;
                return Ok(());
            }
        }
    }
}

async fn enqueue_call(
    state: &AppState,
    run_state: &RunState,
    run_id: Uuid,
    ctx_id: usize,
    op_id: usize,
    call: &Call,
) -> anyhow::Result<()> {
    let task_version = run_state
        .task_versions
        .get(&call.task_id)
        .ok_or_else(|| anyhow::anyhow!("missing task version for {}", call.task_id))?
        .to_string();
    let manifest = db::get_task_manifest(&state.db, &call.task_id, &task_version).await?;

    validate_manifest(&manifest)?;

    let lease_ms = 60_000u64;
    let lease_expires_at = chrono::Utc::now() + chrono::Duration::milliseconds(lease_ms as i64);
    db::upsert_run_node(
        &state.db,
        run_id,
        op_id,
        ctx_id,
        "queued",
        Some(lease_expires_at),
    )
    .await?;

    let mut redis = state.redis.clone();
    let (input_values, input_hashes, input_refs) = resolve_inputs_for_message(
        &mut redis,
        run_id,
        ctx_id,
        &call.inputs,
        state.inline_input_limit,
        state.object_store.as_ref(),
    )
    .await?;

    let pool = pool_for_manifest(&manifest);
    if !db::has_worker(&state.db, &pool, &manifest.resource_class).await? {
        return Err(anyhow::anyhow!(
            "no workers available for pool {pool} resource_class {}",
            manifest.resource_class
        ));
    }

    let msg = QueueMessage {
        run_id,
        op_id,
        ctx_id,
        task_id: call.task_id.clone(),
        task_version,
        input_ids: call.inputs.clone(),
        lease_ms,
        input_values,
        input_hashes: Some(input_hashes),
        input_refs,
    };
    redis_store::queue_task(&mut redis, &pool, &manifest.resource_class, &msg).await?;
    redis_store::add_event(
        &mut redis,
        run_id,
        &serde_json::json!({
            "event": "queued",
            "op_id": op_id,
            "ctx_id": ctx_id,
            "task_id": call.task_id,
        }),
    )
    .await?;

    Ok(())
}

async fn resolve_inputs_for_message(
    redis: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
    input_ids: &[usize],
    inline_limit: usize,
    object_store: Option<&object_store::ObjectStore>,
) -> anyhow::Result<(
    Option<Vec<JsonValue>>,
    Vec<String>,
    Option<Vec<Option<ValueRef>>>,
)> {
    let mut raw_values = Vec::with_capacity(input_ids.len());
    for id in input_ids {
        let raw = redis_store::get_value_raw(redis, run_id, ctx_id, *id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing input value"))?;
        raw_values.push(raw);
    }

    let input_hashes = raw_values
        .iter()
        .map(|raw| format!("sha256:{:x}", sha2::Sha256::digest(raw.as_bytes())))
        .collect::<Vec<_>>();

    let inline_values =
        if inline_limit > 0 && raw_values.iter().all(|raw| raw.len() <= inline_limit) {
            let mut parsed = Vec::with_capacity(raw_values.len());
            for raw in &raw_values {
                parsed.push(serde_json::from_str(raw)?);
            }
            Some(parsed)
        } else {
            None
        };

    let input_refs = if let Some(store) = object_store {
        let mut refs = Vec::with_capacity(raw_values.len());
        for (idx, raw) in raw_values.iter().enumerate() {
            let needs_ref = if inline_limit == 0 {
                true
            } else {
                raw.len() > inline_limit
            };
            if needs_ref {
                let hash = input_hashes.get(idx).map(|h| h.as_str()).unwrap_or("");
                let value_ref = store.put_value(hash, raw.as_bytes()).await?;
                refs.push(Some(value_ref));
            } else {
                refs.push(None);
            }
        }
        Some(refs)
    } else {
        None
    };

    Ok((inline_values, input_hashes, input_refs))
}

fn pool_for_manifest(manifest: &namu_proto::TaskManifest) -> String {
    if manifest.requires_gpu {
        return "gpu".to_string();
    }
    match manifest.trust {
        TaskTrust::Trusted => "trusted".to_string(),
        TaskTrust::Restricted => "restricted".to_string(),
        TaskTrust::Untrusted => "wasm".to_string(),
    }
}

fn validate_manifest(manifest: &namu_proto::TaskManifest) -> anyhow::Result<()> {
    if manifest.trust == TaskTrust::Untrusted && manifest.runtime != TaskRuntime::Wasm {
        return Err(anyhow::anyhow!("untrusted tasks must use wasm runtime"));
    }
    if manifest.runtime == TaskRuntime::Wasm && manifest.trust != TaskTrust::Untrusted {
        return Err(anyhow::anyhow!("wasm runtime requires trust=untrusted"));
    }
    if manifest.requires_gpu {
        if manifest.runtime == TaskRuntime::Wasm {
            return Err(anyhow::anyhow!("gpu tasks must use native runtime"));
        }
        if manifest.trust == TaskTrust::Untrusted {
            return Err(anyhow::anyhow!("gpu tasks cannot be marked untrusted"));
        }
    }
    Ok(())
}

pub async fn apply_task_output(
    state: &AppState,
    run_id: Uuid,
    op_id: usize,
    ctx_id: usize,
    output_json: JsonValue,
) -> anyhow::Result<()> {
    let run_state = get_run_state(state, run_id).await?;
    let workflow = run_state.workflow.clone();
    let operation = workflow
        .operations
        .get(op_id)
        .ok_or_else(|| anyhow::anyhow!("invalid op id"))?;
    let call = operation
        .call
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("op has no call"))?;

    let task_version = run_state
        .task_versions
        .get(&call.task_id)
        .ok_or_else(|| anyhow::anyhow!("missing task version for {}", call.task_id))?
        .to_string();
    let manifest = db::get_task_manifest(&state.db, &call.task_id, &task_version).await?;

    let mut redis = state.redis.clone();

    match manifest.task_kind {
        TaskKind::Stream => {
            let items = output_json
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("stream task output must be array"))?;

            for item in items {
                let child_ctx = run_state
                    .next_ctx_id
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                db::create_context(&state.db, run_id, child_ctx, Some(ctx_id)).await?;
                redis_store::create_context(&mut redis, run_id, child_ctx, Some(ctx_id)).await?;
                store_outputs(&mut redis, run_id, child_ctx, &call.outputs, item).await?;

                if let Some(next) =
                    next_op_id(&mut redis, run_id, child_ctx, &operation.next).await?
                {
                    drive_until_call(state, run_id, child_ctx, next, Some(op_id)).await?;
                }
            }
            db::finish_context(&state.db, run_id, ctx_id).await?;
        }
        _ => {
            store_outputs(&mut redis, run_id, ctx_id, &call.outputs, &output_json).await?;
            if let Some(next) = next_op_id(&mut redis, run_id, ctx_id, &operation.next).await? {
                drive_until_call(state, run_id, ctx_id, next, Some(op_id)).await?;
            } else {
                db::finish_context(&state.db, run_id, ctx_id).await?;
            }
        }
    }

    Ok(())
}

async fn store_outputs(
    redis: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
    outputs: &[usize],
    output_json: &JsonValue,
) -> anyhow::Result<()> {
    if outputs.is_empty() {
        return Ok(());
    }
    if outputs.len() == 1 {
        redis_store::set_value(redis, run_id, ctx_id, outputs[0], output_json).await?;
        return Ok(());
    }

    let array = output_json
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("output must be array for multiple outputs"))?;
    if array.len() != outputs.len() {
        return Err(anyhow::anyhow!("output arity mismatch"));
    }
    for (idx, val) in array.iter().enumerate() {
        let out_id = outputs[idx];
        redis_store::set_value(redis, run_id, ctx_id, out_id, val).await?;
    }
    Ok(())
}

async fn apply_literals(
    redis: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
    op: &Operation,
) -> anyhow::Result<()> {
    for lit in &op.literals {
        let value = parse_literal(&lit.value);
        redis_store::set_value(redis, run_id, ctx_id, lit.output, &value).await?;
    }
    Ok(())
}

async fn apply_phis(
    redis: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
    op: &Operation,
    pred: Option<usize>,
) -> anyhow::Result<()> {
    let Some(pred) = pred else {
        return Ok(());
    };
    for phi in &op.phis {
        let entry = phi
            .from
            .iter()
            .find(|(from_op, _)| *from_op == pred)
            .ok_or_else(|| anyhow::anyhow!("phi missing predecessor"))?;
        let val_id = entry.1;
        let val = redis_store::get_value(redis, run_id, ctx_id, val_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing value for phi"))?;
        redis_store::set_value(redis, run_id, ctx_id, phi.output, &val).await?;
    }
    Ok(())
}

async fn next_op_id(
    redis: &mut ConnectionManager,
    run_id: Uuid,
    ctx_id: usize,
    next: &Next,
) -> anyhow::Result<Option<usize>> {
    match next {
        Next::Jump { next } => Ok(Some(*next)),
        Next::Branch {
            var,
            true_next,
            false_next,
        } => {
            let cond = redis_store::get_value(redis, run_id, ctx_id, *var)
                .await?
                .ok_or_else(|| anyhow::anyhow!("missing branch value"))?;
            let cond_bool = cond
                .as_bool()
                .ok_or_else(|| anyhow::anyhow!("branch value not bool"))?;
            Ok(Some(if cond_bool { *true_next } else { *false_next }))
        }
        Next::Return { var: _ } => Ok(None),
    }
}

fn parse_literal(raw: &str) -> JsonValue {
    match raw {
        "true" => JsonValue::Bool(true),
        "false" => JsonValue::Bool(false),
        "()" => JsonValue::Null,
        _ => {
            if let Ok(n) = raw.parse::<i32>() {
                JsonValue::Number(n.into())
            } else {
                JsonValue::String(raw.trim_matches('"').to_string())
            }
        }
    }
}

async fn get_run_state(state: &AppState, run_id: Uuid) -> anyhow::Result<RunState> {
    let runs = state.runs.read().await;
    runs.get(&run_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("run state not found"))
}
