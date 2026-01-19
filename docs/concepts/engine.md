# Engine

The engine interprets workflow IR, manages contexts, and dispatches task calls. It is designed to be replaceable for different runtime strategies.

## Engine responsibilities
- Walk operations in the IR.
- Resolve inputs from the value tree.
- Dispatch task calls and wait for outputs.
- Advance contexts and track run progress.

## Current implementations
- **EngineKernel** (in `namu-engine`): a reusable interpreter that drives execution until it needs to dispatch a task or return.
- **SimpleEngine**: a single-process engine used for local testing and examples.
- **OrchestratorEngine**: used by `namu-master` to plan work and enqueue tasks.

## Context management
`namu-engine` abstracts context storage behind a trait so engines can plug in different backends (in-memory, Redis-backed, or cached hybrids).

## Extending the engine
To add a new runtime engine:
1. Implement the engine trait in `namu-engine`.
2. Provide a context manager that matches your storage and caching strategy.
3. Wire task dispatch to your execution backend (local, distributed, GPU, or WASM).
