# Value Tree Acceleration (Worker-side caching + inline inputs)

This document proposes protocol and runtime changes to reduce latency when executing tasks by reusing the persistent value tree on workers and inlining inputs in queue messages.

## Goals
- Reduce per-task latency by avoiding repeated Redis fetches.
- Preserve immutability and determinism of the value tree.
- Keep the orchestrator authoritative while enabling worker-side caching.
- Support large values without blowing up queue payload size.

## Non-goals
- Full offloading of scheduling or branch logic to workers (for now).
- Changing the core IR format or control-flow semantics.

## Current flow (baseline)
1. Orchestrator enqueues a `QueueMessage` with `run_id`, `ctx_id`, and `input_ids`.
2. Worker fetches each input value from Redis (`values:{run_id}:{ctx_id}`), runs the task, and posts outputs.
3. Orchestrator writes outputs into Redis and advances the run.

## Proposed changes
### Phase 1: Inline inputs in queue messages
**Change:** include resolved input values in the queue payload so the worker can skip Redis reads.

New fields (optional):
- `input_values`: ordered list of JSON values matching `input_ids`.
- `input_sizes`: optional byte sizes for observability.
- `input_refs`: optional external references for large values (see Phase 2).

Worker behavior:
- If `input_values` present, use them directly.
- Else fall back to Redis reads (backward compatibility).

Orchestrator behavior:
- Resolve values before enqueuing.
- For large inputs, omit `input_values` and include `input_refs` instead.

### Phase 2: Worker cache with content addressing
**Change:** add value hashes to enable worker-side cache hits and skip both Redis and inline payloads.

New fields (optional):
- `input_hashes`: list of hashes aligned to `input_ids` (e.g. `sha256:<hex>`).

Worker behavior:
- Maintain an LRU cache keyed by hash.
- If a hash is in cache, use cached value.
- If not cached, use inline `input_values` or fetch from Redis, then cache.

Orchestrator behavior:
- Include `input_hashes` whenever possible.
- Optionally omit `input_values` when all inputs are likely cached.

### Phase 3: External value references for large payloads
**Change:** allow storing large values in object storage and sending references.

New fields (optional):
- `input_refs`: list of `{ ref, hash, size, codec }` entries.
  - `ref`: object store URL or key
  - `hash`: content hash
  - `size`: bytes
  - `codec`: e.g. `zstd`, `gzip`, `none`

Worker behavior:
- Download and decode value if not cached.
- Cache by hash.

## Protocol changes (QueueMessage)
Add the following optional fields:

```json
{
  "run_id": "<uuid>",
  "op_id": 12,
  "ctx_id": 42,
  "task_id": "add",
  "task_version": "0.1.0",
  "input_ids": [0, 1],
  "lease_ms": 60000,
  "input_values": [1, 2],
  "input_hashes": ["sha256:...", "sha256:..."],
  "input_refs": [null, {"ref":"s3://...","hash":"sha256:...","size":1234,"codec":"zstd"}]
}
```

Rules:
- `input_values`, `input_hashes`, and `input_refs` are aligned by index to `input_ids`.
- At least one of `input_values` or `input_refs` should be provided if cache misses are expected.
- If a value is provided inline, its `input_refs[i]` can be `null`.

## Implementation plan
1. **Proto updates**
   - Extend `namu_proto::QueueMessage` with optional fields.
2. **Orchestrator**
   - Resolve inputs before enqueueing.
   - Optionally hash values and attach `input_hashes`.
   - Add size threshold for inline values; larger values become refs.
3. **Worker**
   - Add input resolution strategy: cache → inline → ref → Redis.
   - Implement LRU cache with size cap (e.g., 256MB default).
4. **Config**
   - Add env vars for cache size and inline size threshold.

## Compatibility
- Workers ignoring new fields still function (fallback to Redis).
- Orchestrators can incrementally roll out inline values without breaking older workers.

## Risks
- Large inline payloads could overload Redis or queue bandwidth.
- Worker cache growth needs strict limits to avoid memory pressure.
- Hashing costs CPU; may need to batch or sample.

## Testing plan
- Unit: hash + cache resolution (cache hit/miss/evict).
- Integration: enqueue with inline inputs; worker runs without Redis reads.
- E2E: mixed workloads with large values using refs.
