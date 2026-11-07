# Configuration

All settings are environment variables.

Redis holds the control plane's state (published tasks and workflows, members, runs and their
results) and the workers' durable log, which lets a successor take over a dead worker's work.
Workers talk to each other directly over the mesh; Redis is only written behind the critical path.

Redis must keep what it is told: run it with persistence (`appendonly yes`, `appendfsync
everysec` or `always`). On AWS, use **Amazon MemoryDB**, whose writes are durable across a
multi-AZ transaction log, rather than ElastiCache, which can lose recent writes on failover; a
lost log record is a branch lost. Use TLS and an ACL user: `REDIS_URL=rediss://user:password@host:6379/`.
NAMU uses one Redis database and does not need cluster mode; a MemoryDB cluster with one shard
(and replicas for failover) fits.

The orchestrator keeps no state of its own: restarted, it takes up from Redis, gives every member
a new lease, and sends each unfinished run its `Start` again (an owner that has the run ignores
it). Runs go on while it is down, peer to peer; their results wait on the workers until it is
back. Run one orchestrator under a supervisor that restarts it (an ECS service, a Kubernetes
Deployment).

Every member of the mesh (the orchestrator and each worker) listens on one TCP address and
advertises an address every other member can reach. On one machine the defaults work; across
machines, set `NAMU_ADVERTISE_ADDR` to an address the others can reach.

Security (see `security.md`): the API is HTTPS with tokens, the mesh mutual TLS. Without these
settings the orchestrator and workers refuse to start, unless `NAMU_INSECURE=1`.

- `NAMU_TLS_CA`, `NAMU_TLS_CERT`, `NAMU_TLS_KEY`: the cluster's CA, and this member's certificate
  and key (PEM), for the mesh, and for the orchestrator's HTTPS. Every member has them.
- `NAMU_INSECURE=1`: an open API and a plaintext mesh, for a machine nobody else reaches.

## Orchestrator (namu-master)
- `NAMU_TOKENS`: the file of accepted tokens' hashes, `<role> sha256:<hex>` per line.
- `REDIS_URL` (default: `redis://127.0.0.1/`)
- `BIND_ADDR` (default: `0.0.0.0:8080`)
- `ARTIFACTS_DIR` (default: `./data/artifacts`)
- `NAMU_MESH_ADDR` (default: `0.0.0.0:0`, any free port): where it listens in the mesh.
- `NAMU_ADVERTISE_ADDR` (default: `127.0.0.1:<mesh port>`): where workers reach it.

## Worker (namu-worker)
- `NAMU_ORCH_URL` (default: `http://localhost:8080`): `https://...` when secured.
- `NAMU_TOKEN`: a `worker` token.
- `REDIS_URL` (default: `redis://127.0.0.1/`): the durable log.
- `NAMU_TASK_SLOTS` (default: number of CPUs): task calls run at once.
- `NAMU_RUNTIMES` (default: `wasm,native`): runtimes this worker serves. Native tasks are only
  routed to workers on the platform they were built for (`<os>-<arch>`).
- `NAMU_MESH_ADDR` (default: `0.0.0.0:0`, any free port): where it listens in the mesh.
- `NAMU_ADVERTISE_ADDR` (default: `127.0.0.1:<mesh port>`): where other members reach it.
- `WORKER_ID` (default: `<platform>-<random>`): a readable name, shown in run status.
- `ARTIFACT_CACHE` (default: `./data/cache`)

## CLI (namu)
- `NAMU_ORCH_URL`: orchestrator URL; otherwise the one saved by `namu login`.
- `NAMU_TOKEN`: a `client` token, sent with every request.
- `NAMU_TLS_CA`: a CA to trust for the orchestrator's HTTPS, besides the system's roots.
