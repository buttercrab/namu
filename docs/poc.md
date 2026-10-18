# Distributed runtime: status

Status (2026-10-19): workers form a peer-to-peer mesh and run `namu-engine` directly, with crash
recovery through a durable log. This page records what that shows and what it costs. The first proof of concept (#26, #27) put every
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
| Streams | `stream_stats` streams the same 48 documents `doc_stats` gets as a list, through a WASM stream task, and gives the same checksum; each document is analyzed as it arrives. In-process tests check pipelining, the window (backpressure), failures, and resuming a stream after its worker dies. |
| Batching | `label_stats` labels documents with a dynamic batch task (`classify`) and a continuous one (`refine`), both WASM components. In-process tests check that batches form and respect `batch_size`, that a lone item runs at once (or waits for `max_wait`), that item and step failures stay where they belong, that short items overtake a long one in continuous batching, that two runs share steps, and that a batch survives its worker dying. |
| Elastic routing | A branch whose next task no member serves waits until one joins, instead of failing. |
| Crash recovery | `kill -9` of a worker mid-run (`scripts/e2e.sh`): its successor takes over from the log, and the run produces the same result as an uninterrupted run. In-process tests kill a worker, the run's owner, or a successor too, at every moment of a run, and check the outcome. Checkpoint tasks are not repeated. |

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
- **Durability**: workers write what a successor would need to a log in Redis, behind the
  critical path (design §7). Redis is on the data path only as that log.

## Recovery

Design §7, in short:
- **Exactly-once effects.** Every branch has a stable id. Scope owners and run owners count each
  branch once, so work that runs twice (because of recovery) does not change the outcome.
- **Upstream backup.** A handoff, branch end or report is kept by its sender until the receiver
  has logged what it produced; then the sender drops its own record of the branch.
- **Small log.** Nodes are logged only when a record refers to them: the nodes a branch created
  since it arrived, which are exactly the ones a handoff ships.
- **Recovery.** When a worker's heartbeats stop for 5 seconds, the orchestrator names its
  successor (the lowest live id). The successor adopts its runs, scopes and branches from the
  log. Everyone else re-sends what it had not acknowledged, and reads its nodes from the log.
- **Checkpoint tasks** (`"checkpoint": true`) are recorded past before the branch continues, so
  they are never repeated.

Not covered yet: idempotency keys passed to tasks, Redis failure (run Redis with persistence), and
runs whose only server for a task never comes back (they wait).

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
| `stream_stats` | as `doc_stats`, but the documents arrive one by one from a stream task (`stream_docs`) |
| `label_stats` | docs → `classify` (dynamic batch) → `refine` (continuous batch) per doc → `tally` |
| `doc_stats_slow` | `doc_stats` with ~6 s of work; `e2e.sh` kills a worker during it |
| `doc_stats_poison` | short docs crash (trap) their task, failing their branches |

## Measurements (Apple M-series, 14 cores; warm runs; run time as seen by the orchestrator)

| What | Mesh | First PoC (Redis) |
|---|---|---|
| `bench_handoff` on one worker (301 calls) | 1 ms | 2 ms |
| Handoff between two local workers (`bench_handoff`, 200 handoffs) | **~65 µs** (13 ms total) | ~140 µs |
| Handoff macOS ↔ Linux container (Docker Desktop VM network) | **~0.3 ms** | ~0.5 ms |
| `nested_stats` (606 calls) on 3 local workers | 11 ms | 65 ms |
| `bench_local` (1,000-iteration loop, 2,001 calls) | **3 ms** (~1.5 µs/step) | ~40 ms (~20 µs/step) |
| `doc_stats` (51 calls, task-bound), 1 worker vs 3 | ~80 ms vs ~85 ms | ~85 ms vs ~88 ms |

How to read these:
- **A handoff is one message** between workers, carrying the branch and the nodes it needs, plus
  an asynchronous acknowledgement once the receiver has logged it. What remains is TCP on
  localhost, JSON encoding, log writes behind the critical path, and scheduling. Without the log
  (step 2) a handoff took ~50 µs and `nested_stats` 7 ms.
- **Long loops stay fast and small.** A node attaches under the deepest node still holding a live
  value, so a loop's chain stays as short as its live values, and the nodes left behind are swept
  (design §6.2). A 5,000-iteration loop holds a bounded number of nodes; before this, a long loop
  paid for walking an ever longer chain on every lookup.
- **No speedup on one machine is expected** for task-bound work. One worker already uses all
  cores; extra processes on the same host only add coordination.

## Known gaps

1. **No idempotency keys** for tasks with external side effects.
2. **The log keeps a run's branch ends, reports and trail nodes until the run ends.** Workers'
   memory stays flat (nodes are swept), but the log of a run lasting weeks grows; it needs
   compaction.
3. **Values travel as JSON**, inline in messages; there is no binary codec or object-store path
   for large values yet.
4. **Routing is round robin** among the workers that serve a task; load and locality are not
   considered yet (stream credits are local to the source; there is no load feedback).
5. **Signatures are written by hand** in the workflow crate; WIT-typed signatures and generated
   stubs come later.
