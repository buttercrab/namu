# Architecture

NAMU compiles Rust workflows into a JSON IR and executes the graph across workers. At runtime, an orchestrator coordinates durable state and scheduling, while workers execute task artifacts.

## Components
- **namu-macros**: `#[task]` and `#[workflow]` procedural macros that emit a JSON IR graph.
- **namu-flow**: in-memory graph builder used by macros at compile time.
- **namu-core**: shared IR types and runtime traits.
- **namu-engine**: IR interpreter, context management, and execution kernel.
- **namu-master**: orchestrator API that stores metadata and schedules work.
- **namu-worker**: runtime node that loads task artifacts and executes calls.
- **Postgres**: durable storage for tasks, workflows, runs, contexts, and workers.
- **Redis**: queue, run events, and value tree storage.
- **Object store (optional)**: large value storage when inline payloads are disabled or too large.

## Compile-time flow
```
Rust source -> namu-macros -> namu-flow -> JSON IR
```
- Tasks and workflows stay as idiomatic Rust.
- The workflow macro produces a static single-assignment (SSA) graph and serializes it.

## Runtime flow
1. `namu publish` uploads task artifacts and workflow IR to the orchestrator.
2. `namu run` creates a workflow run and the orchestrator plans execution.
3. The orchestrator resolves input values, decides the worker pool, and enqueues work.
4. Workers fetch artifacts, execute tasks, and report outputs.
5. The orchestrator advances the run and emits events.

## Data model (high level)
- **Value tree**: immutable context snapshots stored in Redis.
- **Runs**: workflow instances tracked in Postgres.
- **Artifacts**: task bundles stored on disk and/or object storage.

## Crate layout (current)
```
crates/
  apps/
    cli/     master/     worker/
  libs/
    core/    engine/     flow/     macros/    proto/
```
