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
- `NAMU_OBJECT_STORE_ENDPOINT` (optional): S3-compatible endpoint (e.g. `http://127.0.0.1:9000`)
- `NAMU_OBJECT_STORE_BUCKET` (optional): bucket name (default: `namu`)
- `NAMU_OBJECT_STORE_ACCESS_KEY` (optional): access key (default: `minioadmin`)
- `NAMU_OBJECT_STORE_SECRET_KEY` (optional): secret key (default: `minioadmin`)
- `NAMU_OBJECT_STORE_REGION` (optional): region (default: `us-east-1`)
- `NAMU_OBJECT_STORE_FORCE_PATH_STYLE` (optional): bool, default `true` (MinIO-friendly)

### Worker (namu-worker)
- `NAMU_ORCH_URL` (optional): defaults to `http://localhost:8080`
- `REDIS_URL` (optional): defaults to `redis://127.0.0.1/`
- `WORKER_ID` (optional): auto-generated UUID
- `RESOURCE_CLASS` (optional): defaults to `cpu.small`
- `WORKER_POOL` (optional): execution pool (`trusted`, `restricted`, `wasm`, `gpu`), default `trusted`
- `LABELS_JSON` (optional): JSON map of labels
- `ARTIFACT_CACHE` (optional): defaults to `./data/cache`
- `NAMU_VALUE_CACHE_BYTES` (optional): value cache size (default: 268435456)
- `NAMU_OBJECT_STORE_ENDPOINT` (optional): S3-compatible endpoint (e.g. `http://127.0.0.1:9000`)
- `NAMU_OBJECT_STORE_BUCKET` (optional): bucket name (default: `namu`)
- `NAMU_OBJECT_STORE_ACCESS_KEY` (optional): access key (default: `minioadmin`)
- `NAMU_OBJECT_STORE_SECRET_KEY` (optional): secret key (default: `minioadmin`)
- `NAMU_OBJECT_STORE_REGION` (optional): region (default: `us-east-1`)
- `NAMU_OBJECT_STORE_FORCE_PATH_STYLE` (optional): bool, default `true` (MinIO-friendly)
