# Configuration

All settings are environment variables.

Redis holds the control plane's state: published tasks and workflows, members, and run status.
Workers do not use it; they talk to each other directly over the mesh.

Every member of the mesh (the orchestrator and each worker) listens on one TCP address and
advertises an address every other member can reach. On one machine the defaults work; across
machines, set `NAMU_ADVERTISE_ADDR` to an address the others can reach.

## Orchestrator (namu-master)
- `REDIS_URL` (default: `redis://127.0.0.1/`)
- `BIND_ADDR` (default: `0.0.0.0:8080`)
- `ARTIFACTS_DIR` (default: `./data/artifacts`)
- `NAMU_MESH_ADDR` (default: `0.0.0.0:0`, any free port): where it listens in the mesh.
- `NAMU_ADVERTISE_ADDR` (default: `127.0.0.1:<mesh port>`): where workers reach it.

## Worker (namu-worker)
- `NAMU_ORCH_URL` (default: `http://localhost:8080`)
- `NAMU_RUNTIMES` (default: `wasm,native`): runtimes this worker serves. Native tasks are only
  routed to workers on the platform they were built for (`<os>-<arch>`).
- `NAMU_MESH_ADDR` (default: `0.0.0.0:0`, any free port): where it listens in the mesh.
- `NAMU_ADVERTISE_ADDR` (default: `127.0.0.1:<mesh port>`): where other members reach it.
- `WORKER_ID` (default: `<platform>-<random>`): a readable name, shown in run status.
- `ARTIFACT_CACHE` (default: `./data/cache`)

## CLI (namu)
- `NAMU_ORCH_URL`: orchestrator URL; otherwise the one saved by `namu login`.
