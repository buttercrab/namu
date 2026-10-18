//! Exactly-once effects for at-least-once work items.
//!
//! Processing a work item only creates value-tree nodes (immutable and unreferenced until
//! committed, so a crashed attempt leaves nothing but garbage). Everything other workers can
//! observe is collected into [`Effects`] and applied by one Lua script, together with the ack:
//! a crash before the commit leaves no trace and the item is redelivered; a redelivered item
//! whose effects were already committed is only acked.

use namu_proto::{WorkItem, keys};
use redis::aio::ConnectionManager;
use serde::Serialize;
use uuid::Uuid;

/// Shared-state changes from processing one work item. Context ids are strings so they survive
/// Lua's double-precision numbers.
#[derive(Debug, Default, Serialize)]
pub struct Effects {
    /// New fan-out scopes: `(scope, live)`.
    scope_sets: Vec<(String, usize)>,
    /// Work to hand to other workers: `(queue, item json)`.
    enqueue: Vec<(String, String)>,
    /// Workflow results (JSON).
    results: Vec<String>,
    /// Branch failures.
    failures: Vec<String>,
    /// Collect arrivals: `(scope, collect op, item json)`.
    collects: Vec<(String, usize, String)>,
    /// Ending branches, each as its scope chain (innermost last).
    end_branches: Vec<Vec<String>>,
}

impl Effects {
    pub fn set_scope(&mut self, scope: u64, live: usize) {
        self.scope_sets.push((scope.to_string(), live));
    }

    pub fn enqueue(&mut self, queue: &str, item: &WorkItem) -> anyhow::Result<()> {
        self.enqueue
            .push((queue.to_string(), serde_json::to_string(item)?));
        Ok(())
    }

    pub fn result(&mut self, value: &serde_json::Value) -> anyhow::Result<()> {
        self.results.push(serde_json::to_string(value)?);
        Ok(())
    }

    pub fn failure(&mut self, message: String) {
        self.failures.push(message);
    }

    pub fn collect(
        &mut self,
        scope: u64,
        op: usize,
        item: &serde_json::Value,
    ) -> anyhow::Result<()> {
        self.collects
            .push((scope.to_string(), op, serde_json::to_string(item)?));
        Ok(())
    }

    /// Ends the branch whose context has these scopes.
    pub fn end_branch(&mut self, scopes: &[u64]) {
        self.end_branches
            .push(scopes.iter().map(|s| s.to_string()).collect());
    }

    pub fn merge(&mut self, other: Effects) {
        self.scope_sets.extend(other.scope_sets);
        self.enqueue.extend(other.enqueue);
        self.results.extend(other.results);
        self.failures.extend(other.failures);
        self.collects.extend(other.collects);
        self.end_branches.extend(other.end_branches);
    }
}

/// Where a work item came from; its stream entry id identifies it across redeliveries.
#[derive(Debug, Clone)]
pub struct Delivery {
    pub queue: String,
    pub id: String,
    pub attempts: usize,
}

// KEYS are not declared because they depend on the effects; NAMU targets a single Redis node.
//
// ARGV: 1 run id, 2 effects json, 3 now (ms), 4 queue, 5 entry id, 6 consumer group,
//       7 done set, 8 run state, 9 results, 10 failures, 11 scope key prefix, 12 any-queue.
const COMMIT: &str = r#"
local run, effects, now = ARGV[1], cjson.decode(ARGV[2]), ARGV[3]
local queue, entry, group = ARGV[4], ARGV[5], ARGV[6]
local done, state, results, failures = ARGV[7], ARGV[8], ARGV[9], ARGV[10]
local scope_prefix, any_queue = ARGV[11], ARGV[12]

if redis.call('SADD', done, queue .. '/' .. entry) == 0 then
  redis.call('XACK', queue, group, entry)
  return 0
end

for _, s in ipairs(effects.scope_sets) do
  redis.call('HSET', scope_prefix .. s[1], 'live', s[2])
end
for _, q in ipairs(effects.enqueue) do
  redis.call('XADD', q[1], '*', 'item', q[2])
end
for _, r in ipairs(effects.results) do
  redis.call('RPUSH', results, r)
end
for _, f in ipairs(effects.failures) do
  redis.call('RPUSH', failures, f)
end
for _, c in ipairs(effects.collects) do
  redis.call('RPUSH', scope_prefix .. c[1] .. ':items', c[3])
  redis.call('HSET', scope_prefix .. c[1], 'collect_op', c[2])
end

-- A branch that ends decrements its innermost scope. The branch that brings a scope to zero
-- schedules the collect continuation if any branch reached the collect; otherwise the scope's
-- own branch has ended too and the decrement moves one scope outwards. Outside every scope it
-- decrements the run, which finishes at zero.
for _, chain in ipairs(effects.end_branches) do
  local i = #chain
  while true do
    if i == 0 then
      if redis.call('HINCRBY', state, 'live', -1) == 0 then
        local status = 'succeeded'
        if redis.call('LLEN', failures) > 0 then status = 'partial_failed' end
        redis.call('HSET', state, 'status', status, 'finished_at_ms', now)
      end
      break
    end
    local scope = chain[i]
    if redis.call('HINCRBY', scope_prefix .. scope, 'live', -1) > 0 then break end
    local op = redis.call('HGET', scope_prefix .. scope, 'collect_op')
    if op then
      redis.call('XADD', any_queue, '*', 'item',
        '{"kind":"collect_ready","run_id":"' .. run .. '","scope":' .. scope .. ',"op":' .. op .. '}')
      break
    end
    i = i - 1
  end
end

redis.call('XACK', queue, group, entry)
return 1
"#;

/// Applies `effects` and acks the delivery, atomically and at most once per delivery.
pub async fn commit(
    redis: &mut ConnectionManager,
    run: Uuid,
    delivery: &Delivery,
    effects: &Effects,
    now_ms: u64,
) -> anyhow::Result<bool> {
    let script = redis::Script::new(COMMIT);
    let applied: i64 = script
        .arg(run.to_string())
        .arg(serde_json::to_string(effects)?)
        .arg(now_ms)
        .arg(&delivery.queue)
        .arg(&delivery.id)
        .arg(keys::CONSUMER_GROUP)
        .arg(keys::run_done(run))
        .arg(keys::run_state(run))
        .arg(keys::run_results(run))
        .arg(keys::run_failures(run))
        .arg(keys::scope_prefix(run))
        .arg(keys::QUEUE_ANY)
        .invoke_async(redis)
        .await?;
    Ok(applied == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The commit script indexes these arrays positionally; context ids must stay strings so
    /// large ids survive Lua's doubles.
    #[test]
    fn effects_match_the_commit_script_contract() {
        let mut effects = Effects::default();
        let big = namu_proto::context_id(4095, 12345);
        effects.set_scope(big, 3);
        effects
            .collect(big, 7, &serde_json::json!({"a": 1}))
            .unwrap();
        effects.end_branch(&[1, big]);
        effects.end_branch(&[]);
        let json: serde_json::Value = serde_json::to_value(&effects).unwrap();
        assert_eq!(json["scope_sets"][0][0], big.to_string());
        assert_eq!(json["scope_sets"][0][1], 3);
        assert_eq!(json["collects"][0][1], 7);
        assert_eq!(json["collects"][0][2], r#"{"a":1}"#);
        assert_eq!(json["end_branches"][0][1], big.to_string());
        assert_eq!(json["end_branches"][1], serde_json::json!([]));
    }
}
