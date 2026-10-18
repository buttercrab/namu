# Configuration

All settings are environment variables.

## Orchestrator (namu-master)
- `REDIS_URL` (default: `redis://127.0.0.1/`)
- `BIND_ADDR` (default: `0.0.0.0:8080`)
- `ARTIFACTS_DIR` (default: `./data/artifacts`)

## Worker (namu-worker)
- `NAMU_ORCH_URL` (default: `http://localhost:8080`)
- `REDIS_URL` (default: `redis://127.0.0.1/`)
- `NAMU_RUNTIMES` (default: `wasm,native`): runtimes this worker serves. Native tasks are only
  routed to workers on the platform they were built for (`<os>-<arch>`).
- `NAMU_CONCURRENCY` (default: number of CPUs): work items processed concurrently.
- `WORKER_ID` (default: `<platform>-<random>`)
- `ARTIFACT_CACHE` (default: `./data/cache`)

## CLI (namu)
- `NAMU_ORCH_URL`: orchestrator URL; otherwise the one saved by `namu login`.
