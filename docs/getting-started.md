# Getting Started

This guide gets you from a clean checkout to a working local run.

## Requirements
- Rust (stable toolchain)
- Docker (for Postgres, Redis, and optional MinIO)

## One-command demo
```bash
./scripts/e2e.sh
```
This script:
- boots Postgres, Redis, and MinIO with Docker,
- runs the orchestrator and a worker locally,
- builds tasks + workflows from `tests/e2e`,
- publishes artifacts, and
- executes a sample workflow to completion.

Ports can be overridden via `PG_PORT`, `REDIS_PORT`, `BIND_PORT`, `MINIO_PORT`, and `MINIO_CONSOLE_PORT`.

## Manual run (step-by-step)
### 1) Start dependencies
```bash
docker compose -f dev/docker-compose.yml up -d
```

### 2) Run the orchestrator (master)
```bash
DATABASE_URL=postgres://namu:namu@127.0.0.1:5432/namu \
REDIS_URL=redis://127.0.0.1/ \
BIND_ADDR=127.0.0.1:8080 \
ARTIFACTS_DIR=./data/artifacts \
NAMU_OBJECT_STORE_ENDPOINT=http://127.0.0.1:9000 \
NAMU_OBJECT_STORE_BUCKET=namu \
NAMU_OBJECT_STORE_ACCESS_KEY=minioadmin \
NAMU_OBJECT_STORE_SECRET_KEY=minioadmin \
NAMU_OBJECT_STORE_FORCE_PATH_STYLE=true \
cargo run -p namu-master
```

### 3) Run a worker
```bash
NAMU_ORCH_URL=http://127.0.0.1:8080 \
REDIS_URL=redis://127.0.0.1/ \
ARTIFACT_CACHE=./data/cache \
WORKER_POOL=trusted \
RESOURCE_CLASS=cpu.small \
NAMU_OBJECT_STORE_ENDPOINT=http://127.0.0.1:9000 \
NAMU_OBJECT_STORE_BUCKET=namu \
NAMU_OBJECT_STORE_ACCESS_KEY=minioadmin \
NAMU_OBJECT_STORE_SECRET_KEY=minioadmin \
NAMU_OBJECT_STORE_FORCE_PATH_STYLE=true \
cargo run -p namu-worker
```

### 4) Build and publish tasks/workflows
Using the built-in fixtures:
```bash
cargo run -p namu-cli -- build --tasks-dir tests/e2e/tasks --workflows-dir tests/e2e/workflows --out-dir tests/e2e/dist
cargo run -p namu-cli -- publish --out-dir tests/e2e/dist
```

### 5) Run a workflow
```bash
cargo run -p namu-cli -- run add_workflow 0.1.0
```
Use `namu status <run_id>` and `namu logs <run_id>` to inspect progress.

## Next steps
- Read `docs/architecture.md` for the runtime model.
- Use `examples/basic` for a minimal code reference.
- Use `examples/advanced` for a full pipeline with stream, batch, and branching.
