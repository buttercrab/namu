# NAMU

Composable pipeline engine for Rust. NAMU turns Rust workflows into a JSON IR at compile time and executes them across workers at runtime.

Status: experimental. APIs and runtime behavior may change.

## What you get
- `#[task]` and `#[workflow]` macros that generate an immutable workflow graph.
- Type-driven fan-out and fan-in: pass a `Vec<T>` where a task takes `T` to fan out, pass items where it takes `Vec<T>` to collect.
- Workers that drive workflows themselves, with an immutable value tree shared through Redis.
- Tasks written once and built as WASM components (run anywhere) or native libraries.
- CLI to build artifacts, publish workflows, and run jobs.

## Quick start
Requirements: Rust (stable) with `wasm32-wasip2`, and `redis-server`.

```bash
./scripts/poc-local.sh
```
This starts Redis, the orchestrator and three workers, publishes `examples/distributed`, and runs a
fan-out/fan-in workflow across the workers. See `docs/poc.md`.

## How it works (high level)
```
Rust source -> namu-macros -> JSON IR -> namu-engine -> workers
                                  |                     ^
                                  +---- namu-master -----+
```

## Examples
- `examples/basic` - a workflow and its tasks, run in-process (`cargo run -p namu-basic`).
- `examples/distributed` - WASM and native tasks across workers.

## Documentation
Start with `docs/README.md` for the doc index.

## Contributing
PRs welcome. See `docs/contributing.md`.

## License
MIT (see Cargo.toml).
