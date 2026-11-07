# Development

## Build and test
```bash
cargo build --workspace
cargo test --workspace --all-targets --all-features
```

## Lint and format
```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo +nightly fmt --all --check
```

## Full verification (CI parity)
```bash
./scripts/verify.sh
```
This runs format, check, clippy, tests, docs, and security checks.

## End-to-end run
```bash
./scripts/e2e.sh
```
Runs the distributed example on a local cluster and checks the results. Needs `redis-server` and
the `wasm32-wasip2` target. Use `REDIS_PORT` and `ORCH_PORT` to avoid port conflicts.

## Real pipelines
```bash
./scripts/crawl.sh      # a crawler against a site that misbehaves
./scripts/rag.sh        # Wikipedia into Qdrant; CHECK=1 kills a worker and checks recall@5
./scripts/chaos.sh      # kills, a drain, orchestrator and Redis restarts during one run
./scripts/soak.sh       # a long stream: memory and the log stay flat
./scripts/kind.sh       # the images and the Helm chart on a kind cluster
```
See `testing.md`.

## Helpful paths
- `examples/distributed/` - tasks and workflows used by `scripts/e2e.sh`
- `examples/basic/` - a workflow and its tasks run in-process on a `LocalCluster`
- `examples/advanced/` - a labeling pipeline: a stream and continuous batching, in-process
