# Distribution proof of concept

Status: working (2026-10-18), with crash recovery. This is a thin slice of the target
architecture in `design.md`, built to show that distribution works and to get baseline numbers.
Level-2 deduplication, liveness-based placement, Python, and GPU are out of scope.

## What it proves

| Claim | How it is shown |
|---|---|
| Compile once, run everywhere | The same `.wasm` task artifacts run on a macOS worker and on Linux workers (Docker), with no rebuild. |
| Work spreads across machines | Fanned-out items execute on every worker; `calls_by_worker` in the run status shows the split. |
| Values move correctly through the value tree | Per-item branches read the shared `cfg` produced in the parent context, wherever they run (`remote_node_fetches_by_worker`). Results are identical (same checksum) on 1 worker, 3 workers, and macOS + Linux. |
| Per-task runtimes mix | `summarize` is a native library; it is only ever routed to a worker on the platform it was built for. Every other task is WASM. |
| Fan-out / fan-in semantics | Nested fan-out with inner and outer collects; failed branches are dropped from the collect and the run ends `partial_failed`. 100 consecutive stress runs and `scripts/e2e.sh` pass. |
| Crash recovery | `kill -9` of a worker mid-run: its in-flight work is reclaimed and the run produces the same result as an uninterrupted run. Killing the only native worker pauses the run until a replacement starts. Tasks that crash on every attempt are retried, then failed. All in `scripts/e2e.sh`. |

## Architecture of the slice

```
namu CLI ──publish──▶ orchestrator (control plane: registry, runs, status; never per step)
                          │ writes run meta + root node, enqueues root
                          ▼
                    Redis: value-tree nodes, scope counters, work queues
                     ▲   ▲   ▲
        ┌────────────┘   │   └────────────┐
   worker (macOS)    worker (Linux)    worker (Linux)
   wasm + native      wasm              wasm
```

- **Workers run the kernel themselves.** A context advances locally until it needs a runtime the
  worker lacks (it is then handed off on `namu:q:rt:<runtime>`) or it fans out (children go on
  `namu:q:any`). The orchestrator only starts runs and answers status queries.
- **The value tree is immutable and distributed.** Each step creates a node (parent pointer, fan-out
  scopes, values). Nodes are written behind the critical path; before any handoff the worker
  flushes, so the receiver can read every ancestor. Nodes from other workers are fetched once and
  cached; they never change.
- **Fan-out and fan-in are type-driven.** Passing `Vec<T>` to a task parameter `T` fans out;
  passing per-item values to `Vec<T>` collects. rustc resolves this through the `IntoArg` trait.
- **Fan-in termination** uses a `live` counter per scope. The branch that brings it to zero fires
  the collect continuation; if no branch reached the collect, the scope's own branch ends and the
  count moves one scope out (this is what makes nesting and failures work).
- **Tasks** are written once with `namu_guest::task!` and built either as a WASM component
  (`namu:task` WIT world, JSON payloads) or as a native library (C ABI v2: the task runs exactly
  once per call; the library owns its output buffer). Workflows reference tasks through bodiless
  `#[task]` signatures, so the workflow crate never links task code.

## Recovery

Work items are delivered at least once; their effects are applied exactly once.

- **A work item is a transaction.** Processing drives the context locally and creates value-tree
  nodes, which nothing references until the item commits. Everything others can observe
  (enqueued work, scope and run counters, results, failures, collected items) is applied by one
  Lua script together with the ack, and recorded in a per-run "done" set. A crash before the
  commit leaves nothing visible; a duplicate commit is only acked.
- **Leases.** A delivered item stays pending in Redis until it commits. Workers renew the leases
  of items they are working on (`XCLAIM`); any worker takes over items whose lease expired
  (`XAUTOCLAIM`, `NAMU_LEASE_MS`, default 30 s).
- **Recompute from lineage.** A reclaimed item restarts from its context, which is durable
  (written before the item that references it was committed). Tasks in the lost segment run
  again, so tasks run at least once.
- **Poison items.** An infrastructure failure (trap, crash, missing artifact) leaves the item
  pending for retry. After 3 attempts the branch fails and the run continues.
- **Checkpoint tasks.** `"checkpoint": true` in a task manifest commits right after the task, so a
  later crash resumes after it instead of re-running it. A crash during the task itself still
  re-runs it.

What recovery does not cover yet: idempotency keys passed to tasks (needed for exactly-once
external side effects), Redis failure (Redis must be run with persistence), and runs whose
required runtime never comes back (they wait).

## Running it

```bash
./scripts/poc-local.sh                          # 3 local workers, doc_stats workflow
WORKFLOW=nested_stats RUNS=5 ./scripts/poc-local.sh
./scripts/poc-cross.sh                          # Redis, orchestrator, 2 Linux workers in Docker + a macOS worker
./scripts/e2e.sh                                # asserted end-to-end check (also runs in CI)
```

Requirements: `redis-server`, the `wasm32-wasip2` Rust target, and Docker for `poc-cross.sh`.

Workflows in `examples/distributed/workflows`:

| Workflow | Shape |
|---|---|
| `doc_stats` | 48 docs → `analyze` per doc (WASM) → collect → `summarize` (native) |
| `nested_stats` | docs → chunks per doc → `analyze` per chunk → collect per doc → collect all |
| `doc_stats_partial` | as `doc_stats`, but short docs fail and are dropped |
| `bench_local` | `while` loop of trivial WASM calls on one worker |
| `bench_handoff` | alternates WASM and native calls; with them on different workers, every step is a handoff |
| `doc_stats_slow` | `doc_stats` with ~6 s of work, used to kill a worker mid-run |
| `doc_stats_poison` | short docs crash their task on every attempt; they are retried, then failed |

## Measurements (Apple M-series, 14 cores; warm runs)

| What | Result |
|---|---|
| Per local step, shallow tree (`bench_handoff` on one worker, 301 calls) | **~7 µs** (2 ms total) |
| Per local step, 1,000-iteration loop (`bench_local`, 2,001 calls) | **~20 µs** (40 ms total); grows with tree depth, see gap 7 |
| Handoff between two local workers (`bench_handoff`, 200 handoffs) | **~140 µs**, including the atomic commit (~20 µs) |
| Handoff macOS ↔ Linux container (Docker Desktop VM network) | **~0.5 ms** |
| `doc_stats` (51 calls), 1 worker vs 3 workers on the same machine | ~85 ms vs ~88 ms |

Cold start (first run on fresh workers) adds about 100–250 ms: artifact download and WASM
compilation.

How to read these:
- **Per-step overhead is within the design target (≤ 10 µs) while the tree is shallow**, but long
  loops pay for walking the parent chain on every lookup (gap 7). Getting here took pooling WASM
  instances and running calls on the worker thread instead of the blocking pool.
- **A handoff costs several Redis round trips** (flush, enqueue, consumer wake-up, ancestor
  fetch), not the single round trip the design targets.
- **No speedup on one machine is expected.** One worker already uses all cores; extra processes on
  the same host only add coordination. Showing scale-out needs separate machines.

## Known gaps

1. **Recovery limits** (see Recovery): no idempotency keys for tasks, Redis must persist.
2. **Fan-out over an empty list ends the branch**, instead of producing an empty collect.
3. **Giving up on a poison item needs its context node**; if that node is lost, the item cannot
   be failed cleanly.
4. **Values travel inline in Redis nodes**; there is no object-store path for large values yet.
5. **The `SimpleEngine` (in-process) rejects fan-out and collect.**
6. **No GC**: run data stays in Redis, and workers cache run state forever.
7. **Lookups walk the parent chain** (O(depth)), not the O(log depth) persistent structure of
   `DynamicContextManager`.
8. **Signatures are written by hand** in the workflow crate; WIT-typed signatures and generated
   stubs are the next step.

## Next steps (suggested order)

1. Idempotency keys for tasks (run + context + op), for exactly-once external side effects.
2. Handoff cost: send the needed ancestor nodes with the work item and combine flush + enqueue
   into one round trip.
3. Per-step cost: binary value codec, batched node writes.
4. Typed WIT signatures and generated workflow stubs.
5. Level-2 deduplication and liveness-based placement.
