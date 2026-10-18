# NAMU Design

Status: agreed direction (2026-09-28; data plane, streams and batching revised 2026-10-18). This
document describes where NAMU is going, not what is implemented today. See "Gap vs. current
implementation" at the end for the delta.

## 1. Goal

A **fast, multi-machine workflow engine** programmed with a **workflow DSL**, for:

- data collection (crawling, API ingestion, scraping)
- data pipelines for AI modeling (cleaning, embedding, scoring, indexing)

Users: the author, AI data scientists, and people coming from other workflow engines (Airflow,
Beam, Flink, Ray). NAMU runs as a **service** accessed through an API.

Runs can be long: a crawl or a labeling job may run for days or weeks. When a run is too slow,
the answer is to add machines while it runs.

## 2. Principles

1. **The DSL is orchestration only.** A workflow describes how tasks connect. Tasks are opaque units
   that may be written in Rust, Python, or anything else that speaks the task protocol.
2. **Compile once, ship data.** A workflow compiles to IR (data). The IR is deployed to any number of
   machines without recompiling.
3. **Parallelism comes from data dependencies.** The engine sees the whole graph, so anything whose
   inputs are ready may run, regardless of statement order.
4. **Values are immutable and live in a value tree, whose purpose is deduplication.** Every
   fan-out (a list or stream passed where an element is expected) creates sibling child
   contexts. A value
   is stored once, and computed once, in the shallowest context that has all of its inputs.
   Branches read shared values through their ancestors. Immutability also makes this safe under
   parallelism, recovery and GC.
5. **Near-native speed on the common path.** Framework overhead should be negligible next to the
   tasks themselves. Durability must not put synchronous I/O on every step.
6. **Durable by default, via lineage.** Crashes are recovered by recomputing from immutable
   inputs, and synchronous persistence happens only where side effects require it.
7. **Values move peer to peer.** Workers push work and values directly to the next worker. Shared
   storage is a cache and a durability log, never the hub every step passes through.
8. **Elastic.** Machines join and leave a running workflow. Nothing about a run is tied to the
   set of machines it started on.

## 3. Non-goals (for now)

- Untrusted code. Task code is trusted, so there is no sandbox and no trust tiers. (WASM is
  used as a portable build target, not as a sandbox.)
- Task marketplace or public registry.
- Arbitrary Rust expressions in workflows (see §4.3).
- Keyed joins or shuffles across streams (future work).

## 4. Workflow DSL

### 4.1 What a workflow is

A Rust function annotated with `#[workflow]`. The macro reads its syntax and emits IR; the body
never runs as Rust.

### 4.2 Control flow

All structured control flow is supported: `if`/`else`, `match`, `while`, `break`, `continue`,
early `return`, and calling another workflow as a sub-workflow. Parallel iteration is expressed
through types rather than a loop (§4.5).

Inside a fanned-out branch, `return` **drops that item**. It acts as a filter: the item is simply
absent from any later fan-in.

### 4.3 Expressions

The engine cannot run arbitrary Rust, so an expression in a workflow must be one of:

- a **task call** (executed by a worker),
- a **built-in** evaluated by the engine: literals, arithmetic, comparisons, boolean logic, field
  and index access, tuple construction and destructuring, string formatting,
- a **fan-in** reducer (§4.5).

Anything else is a compile error with a message that says "move this into a task".

### 4.4 Errors

A task call yields a result the workflow can inspect (e.g. `if failed(x) { ... }` or `match`).
Retries, fallbacks and dead-letter handling are written in the DSL. An error nobody handles ends
that branch only; sibling branches continue.

Infrastructure failures (worker crash, network loss) are invisible to the DSL. The engine handles
them (§7).

### 4.5 Streams, fan-out, fan-in

Fan-out and fan-in are **type-driven**. There is no loop syntax for parallelism; the types of a
task's parameters decide:

- **Fan-out:** passing a `Vec<T>` (or the output of a **stream task**, i.e. a list whose items
  arrive over time) to a task whose parameter is `T` maps that task over the items. Each item
  gets its own context and the items run in parallel. The rest of the per-item code runs
  per branch as well.
- **Fan-in (collect):** passing per-item values to a task whose parameter is `Vec<T>` collects
  every surviving branch into one list and continues in the parent scope.
- **Nested fan-out:** a per-item task that returns `Vec<U>` fans out again below that item.

```rust
#[workflow]
fn pipeline(source: Source) -> Index {
    let audios = load_audios(source);    // Vec<Audio>
    let clean  = remove_bgm(audios);     // remove_bgm(Audio): fan-out, one branch per audio
    let lang   = detect_language(clean);
    if !is_english(lang) { return; }     // drops this item only
    let emb    = embed(clean);           // per item
    build_index(emb)                     // build_index(Vec<Emb>): fan-in (collect)
}
```

Resolution happens at compile time. The macro cannot see types, so each call goes through a
trait that has one impl for `TracedValue<T>` (a plain call) and one for `TracedValue<Vec<T>>`
(emits a `FanOut`). rustc picks the right one, so a type mismatch is still a compile error.

The cost is readability: where a per-item scope begins is visible only through types. Tooling
(IR visualizer, error messages) should make scopes explicit.

`while` is the sequential loop and may carry state between iterations. The order of items is
not preserved; if a workflow needs order, it sorts by an index at fan-in.

Fan-in kinds:

- **collect**: implicit, by passing to a `Vec<T>` parameter (bounded streams only).
- **reduce**: fold branch results into an accumulator as they arrive. The merge must be
  associative and commutative, so partial reduces can run per machine and then be combined.
  Syntax TBD.
- **window** (later): collect or reduce per N items or T time; required for unbounded streams.

Both bounded (finite input) and unbounded (continuous feed) streams are in scope. Bounded streams
come first, and no design decision may rule out unbounded streams.

## 5. IR

- A workflow is a **control-flow graph of blocks** (branches, loops, fan-out scopes).
- Each block is a **dataflow DAG** of operations. An operation runs once all its inputs exist.
  Independent calls in the same block run in parallel.
- Operation kinds: `Literal`, `Builtin`, `Call`, `Phi`, `FanOut` (entering a stream scope),
  `FanIn` (collect / reduce / window), `SubWorkflow`.
- Phis at a block entry are evaluated **in parallel** (all sources are read before any write).
- Each value carries **liveness** (the op after which it is dead), so the engine can place nodes
  and order chains as in §6.2.
- Literals are serialized values with a type tag, not debug strings.
- Every value id has a declared type or schema, so tasks in different languages can check each
  other at compile or publish time.
- The IR is versioned and language-neutral.

## 6. Execution model

### 6.1 Contexts and the value tree

The value tree exists to **deduplicate** values and computation across branches.

```rust
let a = 1;
let bs = vec![1, 2, 3];
let m = f(a);          // depends only on a -> computed ONCE, in the parent context
let r = g(m, bs);      // g(M, i32): fan-out -> computed per branch (3 times, in parallel)
```

```
        ctx P: a, m          <- shared, stored and computed once
       /    |    \
   C1:b=1 C2:b=2 C3:b=3      <- each branch holds only its own values
     r      r      r
```

**Placement rule (level 2 dedup):** an op's result is attached to the context at the depth of its
deepest input, never to the leaf that happens to request it. The first branch that needs a value
triggers its computation. Sibling branches wait on the same pending value, so it is computed once
for each distinct ancestor. This covers loop-invariant work inside a fan-out, such as loading a
model or fetching a schema.

Only **fan-out** (lists and streams passed where an element is expected) creates sibling contexts. `if`/`else`,
`match` and `while` are ordinary control flow inside a single context: they add new immutable
nodes along that one path, but never create siblings.

A **context** is one execution path. Each context node holds:

- a **parent** pointer (values are looked up through ancestors),
- the values it **adds** (the tree is immutable: a node is never modified after it is sealed),
- a **scope** pointer to its nearest enclosing fan-out context, or a stack of them for nested
  fan-outs.

A new node is created when a step produces values. Lookups use the persistent structure
(binary-lifting ancestors plus a persistent map from value id to depth), so each lookup is
O(log depth) and nothing is copied.

Consequences:

- Siblings share everything written before they split.
- Parallel steps in one context never race, because each writes its own node.
- Loop iterations create new nodes rather than overwriting.
- A node is freed once nothing references it and it has no children.

### 6.2 Liveness-based placement and ordering

A node is fully immutable, **including its parent**. Once a value is placed, the chain above it
can never be rearranged. Placement is therefore decided when the node is created, using liveness
information the compiler computes from SSA and stores in the IR.

**Rule:** a new node attaches to the **shallowest node that still holds every live value the path
needs**, not blindly to the current leaf. Values that are dead (shadowed, or never used again) do
not need to stay reachable.

```rust
let cfg = load_cfg();          // node A
let mut x = init(cfg);         // node B
while cond(x) {
    let tmp = step1(x);        // dead after this iteration
    x = step2(tmp, cfg);       // old x is shadowed -> dead
}
```

Appending to the leaf every time would grow the chain by one iteration per loop. Dead values
would stay pinned as ancestors, and memory would grow without bound. With liveness-based
placement, the new `x` attaches under A (which holds the still-live `cfg`). The old iteration's
nodes lose their last child and are freed, so depth and memory stay flat regardless of loop
length or stream duration.

**Ordering matters.** A node can only be freed once it has no children. So when several values
are created in one step (literals, phis, tuple outputs), they are chained **longest-lived first**:
long-lived values sit nearer the root and short-lived ones deeper. Short-lived values then never
pin long-lived ones, or the other way round.

**When the ideal order is impossible** (e.g. parallel tasks finish in an unlucky order), nothing
is moved. The engine creates a **new** node at the correct position that references the same
shared value (a cheap reference copy, not a deep copy). The old path is then released. The tree
stays immutable throughout.

This is the same principle as level-2 dedup, seen from the other side: "attach at the depth of
your deepest input" decides where computation happens, and "attach to the shallowest node holding
all live values" decides where the path continues.

### 6.3 Fan-out and fan-in in the tree

```
            ctx F  (opens scope S, owned by the worker that fanned out)
           /  |  \
         C1   C2   C3     one child per item, each carrying a share of S's weight
         |    |
        ...  (C2 fans out again: a new scope, nested in S)
         |
      reaches S's collect -> returns its weight and item to S's owner
```

- Which collect closes which fan-out is fixed by the control-flow graph, so it is found from the
  IR before the run. A scope with no items, or whose items were all dropped, therefore still
  collects `[]` and continues.
- When the scope is over, the collected list is written into a **new child of F**, and the branch
  that fanned out continues from there. Downstream code sees everything from before the fan-out
  plus the collected value, but not per-item values. A scope without a collect simply ends the
  branch that opened it.
- Knowing when a scope is over across machines is distributed termination detection, done by
  **weight throwing**. Each scope has an **owner** (the worker that opened it, encoded in the
  scope's id). A scope has a fixed total weight, and its branches start with shares of it. Weight
  moves with the work: a handoff carries the branch's share, and a branch that ends sends its
  share back to the owner, with its collect item if it has one. Creating branches never sends a
  message to the owner, so no message ordering between workers can end a scope early, and no
  global lock is needed.
- A branch that reports a result or a failure to the run's owner sends its weight with the report,
  and the owner returns it to the scope once the report is recorded. So a run cannot finish while
  a report is still in flight.
- Weights are integers from a pool of 2^62 per scope, so accounting is exact. A source that could
  exhaust its share (a stream running for weeks) borrows more from the owner.
- Weight returns can be batched per worker. For reduce, each worker folds locally and sends one
  partial result with the summed weight.

### 6.4 Hybrid execution

Workers run the IR interpreter (the kernel) themselves. A context advances **locally** through as
many steps as it can: builtins, literals, phis, and calls to tasks loaded on this worker cost no
hop at all. It moves to another worker only at a **boundary**:

- the next task isn't available here (a different runtime or platform, or not loaded),
- the next task needs resources this worker lacks (a GPU, memory),
- the next task is a batch task whose batch is gathered elsewhere (§6.6),
- this worker is overloaded and another has free capacity.

At a boundary the worker **pushes a handoff** directly to the chosen worker (§10.2). Fan-out
children are handoffs too, and are spread across workers the same way.

This gives both properties the use cases need: cheap steps never pay for the network, and
expensive steps (GPU, batch) have a home where work gathers.

### 6.5 Streams

A **stream task** yields items over time; it does not return a list. Each item becomes a child
context of the fan-out scope **as soon as it is yielded**. Streams are pipelined: item 1 can be at
stage 3 while item 50 is at stage 1.

- **Backpressure.** A source starts a new branch only with a credit (§10.3), and the credit
  comes back when a branch ends. When the downstream is saturated, branches end more slowly, and
  the source is not resumed. A slow GPU slows the crawler instead of filling memory.
- **Resumable sources.** A source that runs for weeks must survive a crash without replaying
  from the start. A stream task may yield a **cursor** with its items (an offset, a page token,
  a timestamp). The cursor is persisted with the scope, and after a crash the source restarts
  from the last persisted cursor. Item ids are deterministic (scope + sequence number), so items
  that are yielded again are recognized as duplicates.
- **Bounded vs unbounded.** A bounded stream ends and its scope can be collected. An unbounded
  stream never ends, so it can only be consumed per item, or through a window (§4.5).

### 6.6 Batching

A **batch task** is written for many items at once (`fn label(xs: Vec<Item>) -> Vec<Label>`) but
is called per item in the workflow (`label(item)`), like a single task. Batching is a runtime
concern, not a workflow construct:

- Each branch keeps its own context. Branches that reach the same batch task are pushed to a
  worker that hosts it, wait in that worker's batch queue, and are run together. Each branch
  receives its own result and continues independently. It is **not** a fan-in: nothing about
  the scope changes.
- A batch runs when it reaches `batch_size`, or when its oldest item has waited `max_wait` (a
  per-task latency bound).
- To fill batches, routing prefers the batch worker whose next batch is fullest, so items
  concentrate on fewer workers rather than spreading thinly.
- Items from different scopes, and from different runs of the same task version, may share a
  batch. This is how one inference worker serves many concurrent labeling runs.
- A failed batch call fails every branch in it. A task that can fail per item returns a
  per-item result instead.

## 7. Durability

Durability comes from **lineage**: every value is the output of a known task applied to known
immutable inputs.

- **Write-behind**: nodes, scope state and source cursors are persisted to a **durable log**
  asynchronously, and never block the next step.
- **Upstream backup**: a sender keeps each handoff until the receiver acknowledges that the
  state it created from it is persisted. Write-behind is fast, so this window is milliseconds,
  and it is bounded by credits. A crash therefore loses nothing that is not still held by a
  live sender or by the log.
- **Recovery**: when a worker is declared dead (§10.4), senders re-push their unacknowledged
  handoffs to other workers. Contexts resume from their last persisted node, scopes it owned are
  restored on a new owner from the log, and sources restart from their last cursor. Only the
  unpersisted tail is recomputed.
- **Exactly-once effects**: handoff ids and branch ids are deterministic (run, parent context,
  op, item index). Receivers drop duplicate handoffs, and scope owners drop duplicate weight
  returns and collect items. Effects on NAMU state therefore happen once, even though delivery
  is at-least-once.
- **Checkpoint tasks**: tasks with external side effects (writes to DBs or APIs) are marked as
  checkpoints. Their outputs are persisted synchronously before anything downstream runs, so they
  are never re-executed during recovery.
- Task execution is **at-least-once**. Each call gets a deterministic **idempotency key** (run id +
  context path + op id) so side-effecting tasks can deduplicate.
- **Long runs stay small.** With liveness-based placement (§6.2), dead nodes are freed, in the log
  as well as in memory. The persisted state is then the live frontier of the run, not its history,
  so a run's footprint and its recovery time do not grow with its age.

## 8. Values and storage

- The value encoding is abstracted behind a codec (JSON first; msgpack or Arrow later).
- **Zero-copy when possible**: a Rust task feeding a Rust task on the same machine passes the value
  in memory without serializing it.
- Storage tiers:
  1. **Local memory** on the producing worker. This is the authoritative copy while the worker
     lives. A node id encodes the worker that created it, so no directory is needed to find it.
  2. **Durable log** (behind a trait; Redis with persistence first): write-behind copies of
     nodes, scope state and cursors, used for recovery and for reading nodes whose owner is gone.
  3. **S3-compatible object store**: large values, referenced by hash.
- **What a handoff carries** is decided by liveness. Values the receiving segment needs that were
  created in this branch since its last shared ancestor travel **inline**. Values in shared
  ancestors (e.g. a `cfg` read by every branch) travel **by reference**: the receiver fetches a
  missing node once from its owner (or the log) and caches it, since nodes never change.
- Values are content-hashed, so workers can cache them and deduplicate transfers.

## 9. Tasks

- Language-neutral protocol: serialized inputs in, a serialized result or error out, plus task
  metadata (kind, resources, codec).
- Kinds: `single`, `batch` (§6.6), `stream` (§6.5).
- Execution modes: **in-process** (Rust, fastest) and **out-of-process** (a warm process pool or
  sidecar, used for Python).
- Tasks may hold state (a loaded model, connections) via `prepare`, and are reused across calls.
- SDKs generate the protocol glue: Rust `#[task]`, and a Python SDK later.

## 10. Distribution

### 10.1 Planes

- **Control plane (orchestrator)**: stores workflows and tasks, starts runs, tracks membership and
  run status, and publishes the routing table. It is **not** on the per-step path.
- **Data plane (workers)**: run the kernel, execute tasks, hold their nodes in memory, and push
  handoffs to one another directly.
- **Durable log**: written behind by workers, read only on recovery and cache misses (§7, §8).

### 10.2 Handoffs

A handoff is the only message on the per-step path:

```
Handoff { id, run, resume_at: op, parent: node ref, values: inline live values, weight }
```

The receiver creates a node under `parent` holding `values`, and continues the kernel at
`resume_at`. Handoffs between a pair of workers share one connection and are batched when they
queue up. The transport is a trait: TCP or QUIC between machines, and channels between workers in
one process (which is also how a workflow runs locally and in tests: one engine, no special
local mode).

The in-process configuration is the **reference**: every workflow must produce the same result on
it as on any number of machines, and a conformance suite checks this for every example.

### 10.3 Routing and credits

- The orchestrator publishes a versioned **routing table**: for each task, the workers that can
  run it (runtime, platform, resources, whether it is loaded). Workers cache it; a change is pushed
  to them.
- **Credits** bound how many branches are alive, not how many messages a worker holds. A source
  needs a credit to start a branch (§6.5); a handoff of an existing branch is always accepted.
  Limiting handoffs instead would deadlock as soon as two full workers wait on each other, which
  any workflow alternating between two runtimes would do.
- Workers report their load to their peers, piggybacked on the messages they already exchange.
  At a boundary, the sender picks among the workers that can run the next task, preferring (in
  order) one that already holds the context's referenced nodes, then the least loaded.
- **Work stealing**: an idle worker may take parked contexts from a busy one.

### 10.4 Membership and elasticity

- **Join**: a new worker registers its capabilities. The orchestrator adds it to the routing
  table, the new worker reports its (empty) load, and work flows to it. No rebalancing step is
  needed, because contexts move only at boundaries and the newcomer simply becomes a candidate.
- **Leave (drain)**: the worker leaves the routing table, finishes or hands off what it holds,
  transfers ownership of its scopes, flushes its nodes to the log, then deregisters. Nodes it
  created stay readable from the log.
- **Crash**: workers hold a heartbeat lease with the orchestrator. When it expires, the worker is
  declared dead, the routing table moves to a new version without it, and recovery (§7) starts.
  A worker that comes back after being declared dead rejoins as a new member.

So capacity can be added to a slow run while it runs, and removed when it is done, without
stopping the run.

## 11. Service API

API-first: publish workflow and task versions; start runs (by API call, schedule, or event);
query run status, events, and results; stream the results of unbounded runs.
Auth and multi-tenancy come later, but the API shapes should not preclude them.

## 12. Performance targets

- Same machine: framework overhead per step of **≤ ~10 µs**.
- Across machines: **one one-way network message** per handoff; the acknowledgment is
  asynchronous.
- No synchronous durable write on the common path (§7).
- A run's memory and durable footprint stay flat over days of running (§6.2, §7).

## 13. Open questions

1. Deployment: how task code reaches workers (on-demand artifacts, container images, prebuilt
   worker images), and what "fast deploy" means concretely.
2. Placement policy details (§10.3): how locality, load, batch filling and GPU memory are weighed.
3. Syntax for reduce and window (collect is implicit via `Vec<T>` parameters), and how ambiguous
   cases such as `Vec<Vec<T>>` resolve.
4. Windowing semantics for unbounded streams.
5. Type system: how Rust and Python type information is represented and checked in the IR.
6. Upgrading a task or workflow version while a long run is in progress.
7. Level-2 dedup at compile time: hoisting values that depend only on parent-scope inputs out of
   a fan-out, instead of coordinating pending values between siblings at runtime (§6.1).
8. Bounding the duplicate-detection state of unbounded scopes (ids seen by scope owners).

## 14. Gap vs. current implementation

"Today" reflects `namu-engine` (one engine; `LocalCluster` runs it in-process) and the
proof-of-concept worker (`poc.md`), which still runs the distributed data plane until it moves onto
the engine. ✅ marks rows that are done.

| Area | Today | Target |
|------|-------|--------|
| Parallelism | One call per op; a context waits on each call. Parallelism comes only from fan-out | Dataflow within a block |
| Value tree | ✅ Immutable nodes held by their creator and fetched on demand (engine). Lookups walk the parent chain; GC only at run end | O(log depth) lookups via liveness placement, GC |
| Placement | Always appends to the leaf; loops grow the chain without bound | Liveness-based placement, longest-lived-first ordering |
| Dedup | ✅ Constants are hoisted into the root context. Every other op runs per leaf | Results placed at the deepest input's depth; computed once per ancestor (§13 q7) |
| Fan-out / fan-in | ✅ Type-driven via `IntoArg`; collect ops found statically, so an empty or fully dropped scope collects `[]` (engine) | Plus reduce and window |
| `return` in a branch | Not supported | Drops that item (filter) |
| Phis | ✅ Applied in parallel (regression test `loop_phis_are_parallel_assignments`) | |
| Literals | ✅ JSON-encoded; string literals are `String` | Typed IR values |
| Values | JSON at every boundary | Codec abstraction plus types, zero-copy locally |
| Errors in DSL | A failed branch is dropped and recorded in run failures | Results the DSL can inspect |
| Engine | ✅ One engine: workers, handoffs, scope owners, `Transport` trait; in-process transport (`LocalCluster`) is the reference, checked on one worker vs three in every test | Network transport; the worker binary on the engine (§10) |
| Distributed path | ✅ PoC worker: worker-local kernel; every handoff goes through Redis streams (~140 µs) | Peer-to-peer handoffs; Redis only as cache and durable log (§10) |
| Routing | Round robin among the workers that serve a task | Load-aware, locality-aware placement; credits for sources (§10.3) |
| Streams | PoC worker: stream task output is fanned out once complete. Engine: none yet | Pipelined per item, backpressure, resumable cursors (§6.5) |
| Batch | Not supported | Runtime batching across branches and runs, `batch_size` / `max_wait` (§6.6) |
| Task boundary | ✅ `namu-guest` generates the WASM component (WIT) and native C ABI v2; `#[task]` declares signatures only; `FnTasks` for in-process Rust | Typed WIT signatures with generated stubs; Python SDK |
| Runtimes | ✅ Per-task `wasm` or `native`, routed by runtime and platform | Python, containers, GPU |
| Durability | ✅ PoC worker: atomic commits through Redis, lease reclaim, poison items, `checkpoint` tasks. Engine: none yet | Upstream backup + write-behind log; idempotency keys for tasks (§7) |
| Fan-in termination | ✅ Weight throwing with scope owners (engine). PoC worker: `live` counters in Redis | |
| Membership | Static: workers registered at start | Join, drain and crash handling while a run is in progress (§10.4) |
