# NAMU

Composable pipeline engine for Rust. NAMU turns Rust workflows into a JSON IR at compile time and executes them across workers at runtime.

Status: experimental. APIs and runtime behavior may change.

## What you get
- `#[task]` and `#[workflow]` macros that generate an immutable workflow graph.
- Orchestrator + worker runtime backed by Postgres and Redis.
- CLI to build artifacts, publish workflows, and run jobs.
- Deterministic, immutable value propagation with worker-side caching.

## Quick start
Requirements: Rust (stable) and Docker.

```bash
./scripts/e2e.sh
```
This boots the local stack, publishes fixtures from `tests/e2e`, and runs a workflow end-to-end.

## How it works (high level)
```
Rust source -> namu-macros -> JSON IR -> namu-engine -> workers
                                  |                     ^
                                  +---- namu-master -----+
```

## Examples
- `examples/basic` - minimal tasks + workflow.
- `examples/advanced` - ETL, ML, and media pipelines with batch, stream, and branching.

## Documentation
Start with `docs/README.md` for the doc index.

## Contributing
PRs welcome. See `docs/contributing.md`.

## License
MIT (see Cargo.toml).
