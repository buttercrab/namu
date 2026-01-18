# Development

## Build + test
```bash
cargo build --workspace
cargo test --workspace --all-targets --all-features
```

## Lint + format
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
Use `PG_PORT`, `REDIS_PORT`, and `BIND_PORT` to avoid port conflicts.

## Environment variables
### Orchestrator (namu-master)
- `DATABASE_URL` (required): Postgres connection string
- `REDIS_URL` (optional): defaults to `redis://127.0.0.1/`
- `ARTIFACTS_DIR` (optional): defaults to `./data/artifacts`
- `BIND_ADDR` (optional): defaults to `0.0.0.0:8080`
- `NAMU_INLINE_INPUT_LIMIT_BYTES` (optional): max inline input size (default: 262144)

### Worker (namu-worker)
- `NAMU_ORCH_URL` (optional): defaults to `http://localhost:8080`
- `REDIS_URL` (optional): defaults to `redis://127.0.0.1/`
- `WORKER_ID` (optional): auto-generated UUID
- `RESOURCE_CLASS` (optional): defaults to `cpu.small`
- `LABELS_JSON` (optional): JSON map of labels
- `ARTIFACT_CACHE` (optional): defaults to `./data/cache`
- `NAMU_VALUE_CACHE_BYTES` (optional): value cache size (default: 268435456)
