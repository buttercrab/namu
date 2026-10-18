# Engine

`namu-engine` runs workflows. There is one engine; what changes between a test, a laptop and a
cluster is only the transport between workers.

## The kernel
`kernel::drive` interprets the IR along one context. It applies literals and phis as one step
(phis read all their sources before writing, so swaps work), follows control flow, and stops at
the next action:
- `Call`: a task call,
- `FanOut`: one branch per list item,
- `Collect`: this branch reached the collect of its scope,
- `Return`.

It reads and writes values through a `ValueStore`, and knows nothing about where they live.

## Workers
A worker drives branches with the kernel. It runs the tasks it serves itself and moves a branch to
another worker only when the next task is served elsewhere (a handoff). It also:
- holds the value-tree nodes it creates and serves them to other workers on request (a node id
  names its creator);
- owns the fan-out scopes it opens. Branches return their weight to the owner when they end; the
  scope is over when all weight is back, and the branch that fanned out continues with the
  collected list (see `value-tree.md` and `design.md` §6.3);
- for runs it started, records results and failures and finishes the run.

Tasks are provided by a `Tasks` implementation: `FnTasks` for Rust functions in the same process,
and the WASM and native runtimes on workers.

## Streams
A stream task whose output feeds a fan-out runs as the source of the fan-out's scope, on the
worker that owns it: every item it yields becomes a branch at once. The source yields only while
fewer than `window` of the scope's branches are open, so credits never leave the worker. Its
position is logged, so a successor resumes it in the same scope (design §6.5).

## Batches
A branch that calls a batch task queues its input on the worker and waits for its own result.
For each batch task the worker keeps a session per slot, driven in steps: a step admits queued
items, from any branch of any run, up to the session's capacity, and returns those it finished,
each of which resumes its branch at once. Dynamic batching finishes every item it admits;
continuous batching keeps items across steps (design §6.6).

## Local cluster
`LocalCluster` runs any number of workers in one process, with channels as the transport. It is the
reference configuration: a workflow must produce the same results on one worker, on several, and
across machines. The test suite runs every workflow on one worker and on three and compares them.

```rust
let tasks = FnTasks::new().with("add", |(a, b): (i32, i32)| Ok::<_, String>(a + b));
let cluster = LocalCluster::uniform(3, Arc::new(tasks));
let outcome = cluster.run(workflow_ir).await?;
```

## Across machines
The same workers talk over TCP (`TcpTransport`). `namu-worker` runs one engine worker per process,
with the WASM and native task runtimes, and joins the mesh through the orchestrator, which gives it
its id and the member list. The orchestrator is itself a member (id 0): it sends a run to the
worker that will own it and hears back when it is over. `LocalCluster::tcp` runs the same setup on
localhost, and the test suite checks every workflow on it too.

A worker learns about a run the first time it receives work for it: it asks the run's owner for
the run's description and prepares the tasks it uses. When a run is over, its owner asks every
worker to forget it, and each one reports what it did.

## Crashes
Everything that moves responsibility for a branch (a handoff, a branch's end, a report) is kept
by its sender until the receiver has logged what it produced, behind the critical path (the `Log`
trait: Redis on workers, memory in `LocalCluster`). When a worker dies, the control plane names
its successor, which takes over its runs, scopes and branches from the log; every branch has a
stable id, so work that runs twice counts once. `LocalCluster::kill` simulates a crash, and the
recovery tests kill workers at every moment of a run. See `design.md` §7.
