# Architecture

NAMU compiles Rust workflows into a JSON IR. Workers execute that IR directly, running tasks in the
runtime each task declares. See `design.md` for the target design and `poc.md` for what runs today.

## Components
- **namu-macros**: `#[task]` and `#[workflow]`. Workflows become an SSA graph; task parameter types
  decide fan-out and fan-in. A bodiless `#[task]` declares a task implemented elsewhere.
- **namu-flow**: the graph builder used by the macros.
- **namu-core**: IR types, and the scope analysis that pairs each fan-out with its collect.
- **namu-engine**: the engine. Workers drive branches through the IR (the kernel), hold value-tree
  nodes, own the fan-out scopes they open, and send each other handoffs over a `Transport`.
  `LocalCluster` runs workers in one process over channels: the reference configuration, and how
  workflows run locally and in tests.
- **namu-guest**: SDK for writing tasks once and building them as WASM components (`namu:task`
  WIT world) or native libraries (C ABI).
- **namu-master** (orchestrator, control plane): task and workflow registry, starting runs, run
  status. It is not on the per-step path.
- **namu-worker** (data plane): drives contexts through the IR, runs WASM and native tasks, and
  hands work to other workers. It still runs the proof-of-concept data plane (Redis queues); it
  moves onto `namu-engine` with a network transport next.
- **Redis**: registry, value-tree nodes, fan-in scope counters, and work queues.

## Runtime flow
1. `namu publish` uploads task artifacts and workflow IR. Versions are immutable.
2. `namu run` asks the orchestrator to start a run. It writes the run metadata and the root context
   node, then enqueues the root.
3. A worker picks the root up and drives it. Literals, phis and branches are evaluated locally; tasks
   in a runtime the worker supports run locally too.
4. A fan-out creates one child context per item, and any worker can pick them up. A task needing a
   runtime the worker lacks is handed to a worker that has it.
5. Branches meet again at collects. When the last branch of the root ends, the run finishes.

## Data model
- **Value tree**: immutable context nodes (parent, fan-out scopes, values) in Redis, cached on
  workers.
- **Work items**: references to a context and an op, never values.
- **Artifacts**: `tar.zst` bundles (manifest + `.wasm` or native library) stored by the
  orchestrator and cached by workers, verified by checksum.

## Crate layout
```
crates/
  apps/   cli/  master/  worker/
  libs/   core/ engine/  flow/  guest/  macros/  proto/
examples/
  basic/        (in-process, LocalCluster)
  distributed/  (WASM and native tasks on workers)
```
