# Quickstart

## Requirements
- Rust (stable toolchain). Nightly is only required for `cargo fmt` in CI.
- Docker (for local Postgres + Redis).

## One-command demo
```bash
./scripts/e2e.sh
```
This script:
- boots Postgres + Redis with Docker,
- runs the orchestrator and a worker locally,
- builds sample task artifacts and workflow IR,
- publishes them to the orchestrator, and
- runs a workflow to completion.

Ports can be overridden via `PG_PORT`, `REDIS_PORT`, and `BIND_PORT`.

## Manual local run
### 1) Start Postgres + Redis
```bash
docker compose -f dev/docker-compose.yml up -d
```

### 2) Run the orchestrator
```bash
DATABASE_URL=postgres://namu:namu@127.0.0.1:5432/namu \
REDIS_URL=redis://127.0.0.1/ \
BIND_ADDR=127.0.0.1:8080 \
ARTIFACTS_DIR=./data/artifacts \
cargo run -p namu-master
```

### 3) Run a worker
```bash
NAMU_ORCH_URL=http://127.0.0.1:8080 \
REDIS_URL=redis://127.0.0.1/ \
ARTIFACT_CACHE=./data/cache \
RESOURCE_CLASS=cpu.small \
cargo run -p namu-worker
```

### 4) Build + publish tasks/workflows
```bash
namu build   --tasks-dir ./tasks --workflows-dir ./workflows --out-dir ./dist
namu publish --out-dir ./dist
```

### 5) Run a workflow
```bash
namu run <workflow_id> <version>
```
Use `namu status <run_id>` and `namu logs <run_id>` to inspect progress.
