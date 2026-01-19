# NAMU

Composable, AI‑first pipeline engine for Rust. NAMU turns Rust functions into distributed tasks, wires them into immutable workflow graphs at compile time, and executes them across workers at runtime.

**Status:** experimental. APIs and runtime behavior may change.

## Features
- `#[task]` and `#[workflow]` macros that emit a JSON IR graph.
- Orchestrator + workers model with Postgres + Redis backends.
- CLI for building artifacts, publishing workflows, and running jobs.
- Deterministic, immutable value propagation through workflow contexts.

## Architecture (high level)
```
Rust source  ->  namu-macros  ->  JSON IR  ->  namu-engine  ->  workers
                                   |                          ^
                                   +---- namu-master ----------+
```

## Quick start
Requirements: Rust (stable), plus Docker for the local stack.

```bash
# spins up Postgres + Redis, runs master/worker, builds + publishes sample tasks
./scripts/e2e.sh
```

## Local dev loop
```bash
# terminal 1
DATABASE_URL=postgres://namu:namu@127.0.0.1:5432/namu \
REDIS_URL=redis://127.0.0.1/ \
BIND_ADDR=127.0.0.1:8080 \
ARTIFACTS_DIR=./data/artifacts \
cargo run -p namu-master

# terminal 2
NAMU_ORCH_URL=http://127.0.0.1:8080 \
REDIS_URL=redis://127.0.0.1/ \
ARTIFACT_CACHE=./data/cache \
cargo run -p namu-worker
```

```bash
# CLI
namu build   --tasks-dir ./tasks --workflows-dir ./workflows --out-dir ./dist
namu publish --out-dir ./dist
namu run <workflow_id> <version>
```

## Documentation
See `docs/README.md` for full instructions (architecture, CLI, manifests, testing, and contributing).

## Examples
- `examples/basic` — minimal workflow + tasks.
- `examples/advanced` — three themed pipelines (ETL/ML/Media) with stream, batch, branching, loops, and failure handling.

## Contributing
PRs welcome. See `docs/contributing.md` for guidelines.

## License
MIT (see Cargo.toml).
