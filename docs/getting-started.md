# Getting Started

This guide gets you from a clean checkout to a workflow running across several workers.

## Requirements
- Rust (stable) with the WASM target: `rustup target add wasm32-wasip2`
- `redis-server` on your `PATH`
- Docker, only for the cross-platform demo

## One-command demo
```bash
./scripts/poc-local.sh
```
This builds everything, then starts Redis, the orchestrator and three workers on this machine. It
publishes the tasks and workflows from `examples/distributed`, runs `doc_stats`, and prints the run
status, including how many task calls each worker executed.

Variations:
```bash
WORKFLOW=nested_stats RUNS=5 ./scripts/poc-local.sh       # repeat to see warm timings
WORKERS="wasm native" WORKFLOW=bench_handoff ./scripts/poc-local.sh
./scripts/poc-cross.sh                                    # add Linux workers in Docker
```

## Manual run (step-by-step)
### 1) Start Redis and the orchestrator
```bash
redis-server --port 6379 &
REDIS_URL=redis://127.0.0.1:6379/ BIND_ADDR=127.0.0.1:8080 cargo run --release -p namu-master
```

### 2) Start workers (as many as you like; on other machines, set `NAMU_ADVERTISE_ADDR`)
```bash
NAMU_ORCH_URL=http://127.0.0.1:8080 REDIS_URL=redis://127.0.0.1:6379/ \
NAMU_RUNTIMES=wasm,native cargo run --release -p namu-worker
```

### 3) Build and publish tasks and workflows
```bash
cargo run -p namu-distributed-workflows --bin export -- examples/distributed/dist/workflows
cargo run -p namu-cli -- build --tasks-dir examples/distributed/tasks \
  --workflows-dir examples/distributed/dist/workflows --out-dir examples/distributed/dist
NAMU_ORCH_URL=http://127.0.0.1:8080 cargo run -p namu-cli -- publish --out-dir examples/distributed/dist
```

### 4) Run a workflow
```bash
NAMU_ORCH_URL=http://127.0.0.1:8080 cargo run -p namu-cli -- run doc_stats 0.1.0 --wait
```

## Next steps
- `docs/poc.md`: what the distributed runtime shows today, measurements, known gaps.
- `docs/design.md`: the target design.
- `examples/distributed`: tasks (WASM and native) and workflows used above.
