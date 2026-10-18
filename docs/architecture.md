# Architecture

NAMU compiles Rust workflows into a JSON IR. Workers execute that IR directly, running tasks in the
runtime each task declares. See `design.md` for the target design and `poc.md` for what runs today.

## Components
- **namu-macros**: `#[task]` and `#[workflow]`. Workflows become an SSA graph; task parameter types
  decide fan-out and fan-in. A bodiless `#[task]` declares a task implemented elsewhere.
- **namu-flow**: the graph builder used by the macros.
- **namu-core**: IR types, and the scope analysis that pairs each fan-out with its collect.
- **namu-engine**: the engine. Workers drive branches through the IR (the kernel), hold value-tree
  nodes, own the fan-out scopes they open, and send each other handoffs over a `Transport`.
  `LocalCluster` runs workers in one process over channels: the reference configuration, and how
  workflows run locally and in tests.
- **namu-guest**: SDK for writing tasks once and building them as WASM components (`namu:task`
  WIT world) or native libraries (C ABI).
- **namu-master** (orchestrator, control plane): task and workflow registry, membership, starting
  runs, run status. It is a member of the mesh (id 0) but never on the per-step path.
- **namu-worker** (data plane): an engine worker on the TCP mesh, with the WASM and native task
  runtimes.
- **Redis**: the control plane's store (registry, members, run status), and the workers' durable
  log, written behind the critical path and read only to take over from a dead worker.

## Runtime flow
1. `namu publish` uploads task artifacts and workflow IR. Versions are immutable.
2. Workers register with the orchestrator, get an id and the member list, and keep it current
   with heartbeats.
3. `namu run` asks the orchestrator to start a run. It picks a worker to own the run and sends
   it the workflow over the mesh.
4. The owner drives the root branch. Literals, phis and branches are evaluated locally, and so
   are tasks the worker serves. A task served elsewhere moves the branch there (a handoff); a
   fan-out spreads one branch per item across the workers.
5. A worker receiving work for a run it does not know asks the owner for the run's description.
6. Branches end at collects or returns, returning their scope's weight to its owner. When the
   root scope has all its weight back, the run is over: the owner has every worker forget it,
   and reports the outcome to the orchestrator.

## Data model
- **Value tree**: immutable nodes, each held by the worker that created it (its id says which),
  fetched by others on demand and cached. A handoff carries the nodes the receiver needs.
- **Messages**: handoffs, branch ends, reports and node fetches, sent directly between workers.
- **Artifacts**: `tar.zst` bundles (manifest + `.wasm` or native library) stored by the
  orchestrator and cached by workers, verified by checksum.

## Crate layout
```
crates/
  apps/   cli/  master/  worker/
  libs/   core/ engine/  flow/  guest/  macros/  proto/
examples/
  basic/        (in-process, LocalCluster)
  distributed/  (WASM and native tasks on workers)
```
