# Configuration

This document lists runtime configuration for the orchestrator and workers. All settings are environment variables.

## Orchestrator (namu-master)
Required:
- `DATABASE_URL`: Postgres connection string

Common:
- `REDIS_URL` (default: `redis://127.0.0.1/`)
- `ARTIFACTS_DIR` (default: `./data/artifacts`)
- `BIND_ADDR` (default: `0.0.0.0:8080`)
- `NAMU_INLINE_INPUT_LIMIT_BYTES` (default: `262144`)

Object store (optional):
- `NAMU_OBJECT_STORE_ENDPOINT` (example: `http://127.0.0.1:9000`)
- `NAMU_OBJECT_STORE_BUCKET` (default: `namu`)
- `NAMU_OBJECT_STORE_ACCESS_KEY` (default: `minioadmin`)
- `NAMU_OBJECT_STORE_SECRET_KEY` (default: `minioadmin`)
- `NAMU_OBJECT_STORE_REGION` (default: `us-east-1`)
- `NAMU_OBJECT_STORE_FORCE_PATH_STYLE` (default: `true`)

## Worker (namu-worker)
Common:
- `NAMU_ORCH_URL` (default: `http://localhost:8080`)
- `REDIS_URL` (default: `redis://127.0.0.1/`)
- `WORKER_ID` (auto-generated UUID if unset)
- `RESOURCE_CLASS` (default: `cpu.small`)
- `WORKER_POOL` (default: `trusted`)
- `LABELS_JSON` (JSON map of labels)
- `ARTIFACT_CACHE` (default: `./data/cache`)
- `NAMU_VALUE_CACHE_BYTES` (default: `268435456`)

Object store (optional):
- `NAMU_OBJECT_STORE_ENDPOINT`
- `NAMU_OBJECT_STORE_BUCKET`
- `NAMU_OBJECT_STORE_ACCESS_KEY`
- `NAMU_OBJECT_STORE_SECRET_KEY`
- `NAMU_OBJECT_STORE_REGION`
- `NAMU_OBJECT_STORE_FORCE_PATH_STYLE`
