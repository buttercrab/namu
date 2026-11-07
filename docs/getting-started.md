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
A cluster runs secured: HTTPS and tokens for the API, mutual TLS for the mesh
(`docs/operations/security.md`). For a quick try on one machine, `NAMU_INSECURE=1` on the
orchestrator and workers skips step 1 and the TLS and token settings below.

### 1) Make a CA, a member certificate and tokens
```bash
cargo build --release -p namu-cli -p namu-master -p namu-worker
./target/release/namu certs --out ./secrets --host 127.0.0.1 --host localhost
CLIENT=$(./target/release/namu token --role client --tokens ./secrets/tokens)
WORKER=$(./target/release/namu token --role worker --tokens ./secrets/tokens)
export NAMU_TLS_CA=./secrets/ca.pem NAMU_TLS_CERT=./secrets/member.pem NAMU_TLS_KEY=./secrets/member.key
```

### 2) Start Redis and the orchestrator
```bash
redis-server --port 6379 &
REDIS_URL=redis://127.0.0.1:6379/ BIND_ADDR=127.0.0.1:8080 NAMU_TOKENS=./secrets/tokens \
  ./target/release/namu-master
```

### 3) Start workers (as many as you like; on other machines, set `NAMU_ADVERTISE_ADDR`)
```bash
NAMU_ORCH_URL=https://127.0.0.1:8080 REDIS_URL=redis://127.0.0.1:6379/ NAMU_TOKEN=$WORKER \
  NAMU_RUNTIMES=wasm,native ./target/release/namu-worker
```

### 4) Build and publish tasks and workflows
```bash
export NAMU_ORCH_URL=https://127.0.0.1:8080 NAMU_TOKEN=$CLIENT
cargo run -p namu-distributed-workflows --bin export -- examples/distributed/dist/workflows
./target/release/namu build --tasks-dir examples/distributed/tasks \
  --workflows-dir examples/distributed/dist/workflows --out-dir examples/distributed/dist
./target/release/namu publish --out-dir examples/distributed/dist
```

### 5) Run a workflow
```bash
./target/release/namu run doc_stats 0.1.0 --wait
```

## Next steps
- `docs/poc.md`: what the distributed runtime shows today, measurements, known gaps.
- `docs/design.md`: the target design.
- `examples/distributed`: tasks (WASM and native) and workflows used above.
