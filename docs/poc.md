# Distributed runtime: status

Status (2026-10-19): workers form a peer-to-peer mesh and run `namu-engine` directly. This page
records what that shows and what it costs. The first proof of concept (#26, #27) put every
handoff through Redis; its numbers are kept below for comparison. Level-2 deduplication,
liveness-based placement, streams, batching, Python and GPU are not implemented yet.

## What it shows

| Claim | How it is shown |
|---|---|
| Compile once, run everywhere | The same `.wasm` task artifacts run on a macOS worker and on Linux workers (Docker), with no rebuild. |
| Work spreads across machines | Fanned-out items execute on every worker; `calls_by_worker` in the run status shows the split. |
| One engine, any transport | Every workflow in the test suite gives the same outcome on one worker, on three in-process workers, and on three workers over TCP. `scripts/e2e.sh` and `scripts/poc-cross.sh` run the same engine across processes and machines, with the same checksum as the first proof of concept. |
| Values move correctly through the value tree | Per-item branches read the shared `cfg` produced in the parent context, wherever they run. Each worker fetches a shared node once (`remote_node_fetches_by_worker`); a handoff carries the nodes the receiver needs. |
| Per-task runtimes mix | `summarize` is a native library; it is only ever routed to a worker on the platform it was built for. Every other task is WASM. |
| Fan-out / fan-in semantics | Nested fan-out with inner and outer collects; failed branches are dropped from the collect; an empty or fully dropped scope collects `[]`. Scopes end by weight throwing; 64 concurrent runs on four workers end exactly once with the right result. |
| Elastic routing | A branch whose next task no member serves waits until one joins, instead of failing. |

## Architecture

```
namu CLI ──publish/run──▶ orchestrator ─ registry, members, run status (Redis)
                              │ Start ▲ Finished        (mesh peer 0, never per step)
                              ▼       │
   worker (macOS) ◀────────▶ worker (Linux) ◀────────▶ worker (Linux)
   wasm + native     TCP      wasm               TCP      wasm
         ▲─────────────────────────────────────────────────▲
```

- **Workers drive branches themselves** and run the tasks they serve. A branch moves to another
  worker only when its next task is served elsewhere, or when a fan-out spreads its items.
- **Runs are discovered on demand.** The orchestrator sends a run to its owner; any other worker
  that receives work for it asks the owner for its description.
- **Membership** comes from the orchestrator: workers register, heartbeat every second, and get
  the member list back. A worker whose heartbeats stop for 5 seconds is no longer a member.
- **Run end**: when the root scope has all its weight back, the owner has every worker forget the
  run, collects their stats, and sends the outcome to the orchestrator.

## Recovery

Not yet. The first proof of concept recovered from `kill -9` through Redis work queues and an
atomic commit script; those are gone with Redis on the data path. Recovery comes back in the next
step as designed in `design.md` §7: upstream backup, a write-behind log, and deterministic ids.
Until then, a worker crash loses the runs it took part in.

## Running it

```bash
./scripts/poc-local.sh                          # 3 local workers, doc_stats workflow
WORKFLOW=nested_stats RUNS=5 ./scripts/poc-local.sh
./scripts/poc-cross.sh                          # orchestrator + 2 Linux workers in Docker + a macOS worker
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
| `doc_stats_slow` | `doc_stats` with ~6 s of work |
| `doc_stats_poison` | short docs crash (trap) their task, failing their branches |

## Measurements (Apple M-series, 14 cores; warm runs; run time as seen by the orchestrator)

| What | Mesh | First PoC (Redis) |
|---|---|---|
| `bench_handoff` on one worker (301 calls) | 1 ms | 2 ms |
| Handoff between two local workers (`bench_handoff`, 200 handoffs) | **~50 µs** (10 ms total) | ~140 µs |
| Handoff macOS ↔ Linux container (Docker Desktop VM network) | **~0.3 ms** | ~0.5 ms |
| `nested_stats` (606 calls) on 3 local workers | 7 ms | 65 ms |
| `bench_local` (1,000-iteration loop, 2,001 calls) | ~45 ms (~22 µs/step) | ~40 ms |
| `doc_stats` (51 calls, task-bound), 1 worker vs 3 | ~80 ms vs ~85 ms | ~85 ms vs ~88 ms |

How to read these:
- **A handoff is one message** between workers, carrying the branch and the nodes it needs. What
  remains is TCP on localhost, JSON encoding, and scheduling.
- **Long loops still pay for walking the parent chain** on every lookup (gap 4).
- **No speedup on one machine is expected** for task-bound work. One worker already uses all
  cores; extra processes on the same host only add coordination.

## Known gaps

1. **No recovery** (see Recovery).
2. **No idempotency keys** for tasks with external side effects.
3. **Values travel as JSON**, inline in messages; there is no binary codec or object-store path
   for large values yet.
4. **Lookups walk the parent chain** (O(depth)); liveness-based placement will keep chains short.
5. **Routing is round robin** among the workers that serve a task; load and locality are not
   considered yet, and there are no credits.
6. **Signatures are written by hand** in the workflow crate; WIT-typed signatures and generated
   stubs come later.
