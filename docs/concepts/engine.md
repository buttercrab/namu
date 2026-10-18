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

## Local cluster
`LocalCluster` runs any number of workers in one process, with channels as the transport. It is the
reference configuration: a workflow must produce the same results on one worker, on several, and
across machines. The test suite runs every workflow on one worker and on three and compares them.

```rust
let tasks = FnTasks::new().with("add", |(a, b): (i32, i32)| Ok::<_, String>(a + b));
let cluster = LocalCluster::uniform(3, Arc::new(tasks));
let outcome = cluster.run(workflow_ir).await?;
```

## Distributed workers
`namu-worker` still runs the proof-of-concept data plane (Redis queues and commits, `poc.md`) on the
same kernel. It moves onto the engine's workers, with a network transport, in the next step.
