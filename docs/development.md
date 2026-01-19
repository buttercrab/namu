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
Use `PG_PORT`, `REDIS_PORT`, `BIND_PORT`, `MINIO_PORT`, and `MINIO_CONSOLE_PORT` to avoid port conflicts.

## Helpful paths
- `tests/e2e/` - end-to-end fixtures used by `scripts/e2e.sh`
- `examples/basic/` - minimal workflow + tasks
- `examples/advanced/` - extended pipelines and workflow export
