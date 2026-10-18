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

## Helpful paths
- `examples/distributed/` - tasks and workflows used by `scripts/e2e.sh`
- `examples/basic/` - a workflow and its tasks run in-process on a `LocalCluster`
