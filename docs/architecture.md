# Architecture

NAMU compiles Rust workflows into an immutable JSON IR and executes the graph across workers. The runtime uses a control-plane (orchestrator) plus a worker pool.

## Components
- **namu-macros**: `#[task]` and `#[workflow]` procedural macros that emit a JSON IR graph.
- **namu-flow**: graph builder used by macros.
- **namu-engine**: runtime interpreter for the IR graph.
- **namu-master**: orchestrator API; stores task/workflow metadata and run state.
- **namu-worker**: pulls queue messages, loads artifacts, and executes tasks.
- **Postgres**: durable storage for tasks/workflows/runs.
- **Redis**: queue + run values + events.

## Data flow
1. Tasks and workflows are compiled into artifacts + JSON IR.
2. `namu publish` uploads artifacts and workflow IR to the orchestrator.
3. `namu run` creates a run; the orchestrator plans node execution and enqueues work.
4. Workers consume queue messages, load artifacts, execute the task, and report results.
5. The orchestrator updates the run state and emits run events.

## Failure semantics
- Task failures stop the affected branch (value tree leaf).
- A run can complete as `succeeded` or `partial_failed` depending on leaf failures.

## Persistence model (high level)
- **Postgres**: tasks, artifacts, workflows, runs, contexts, run nodes, workers.
- **Redis**: queue, run values, run events.
