# NAMU Design

Status: agreed direction (2026-09-28). This document describes where NAMU is going, not what is
implemented today. See "Gap vs. current implementation" at the end for the delta.

## 1. Goal

A **fast, multi-machine workflow engine** programmed with a **workflow DSL**, for:

- data collection (crawling, API ingestion, scraping)
- data pipelines for AI modeling (cleaning, embedding, scoring, indexing)

Users: the author, AI data scientists, and people coming from other workflow engines (Airflow,
Beam, Flink, Ray). NAMU runs as a **service** accessed through an API.

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

## 3. Non-goals (for now)

- Untrusted code. Task code is trusted, so there is no WASM sandbox and no trust tiers.
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
            ctx F  (scope: stream consumed here)
           /  |  \
         C1   C2   C3     one child per item; scope(Ci) = F
         |    |
        ...  (C2 forks again: F.live += 1)
         |
      reaches FanIn -> result folded into F's accumulator
```

- Each fan-out scope F keeps `live` (open branches) and `ended` (the stream has finished
  producing items).
- The stream emitting an item increments `live`. A branch forking increments it. A branch
  reaching the FanIn, failing, or ending another way decrements it.
- The FanIn completes when `ended && live == 0`. Its result is written into a **new child of F**,
  and execution continues from there. Downstream code sees everything from before the fan-out plus
  the reduced value, but not per-item values.
- Across machines this is distributed termination detection. The plan is credit-based counting
  owned by F's home node, so no global lock is needed. For reduce, each machine does a partial
  fold before sending one combined update.

### 6.4 Scheduling

Workers run the IR interpreter (the kernel) themselves. A context advances locally through as many
steps as possible, and is handed to another machine only when a step needs resources that machine
has (a GPU, a task that isn't loaded locally, or free capacity).

Streams are pipelined: item 1 can be at stage 3 while item 50 is at stage 1.

## 7. Durability

Durability comes from **lineage**: every value is the output of a known task applied to known
immutable inputs.

- **Write-behind**: values are persisted asynchronously and never block the next step.
- **Recovery**: after a node failure, find each lost value's nearest persisted ancestors and
  recompute only the missing part.
- **Checkpoint tasks**: tasks with external side effects (writes to DBs or APIs) are marked as
  checkpoints. Their outputs are persisted synchronously before anything downstream runs, so they
  are never re-executed during recovery.
- Task execution is **at-least-once**. Each call gets a deterministic **idempotency key** (run id +
  context path + op id) so side-effecting tasks can deduplicate.

## 8. Values and storage

- The value encoding is abstracted behind a codec (JSON first; msgpack or Arrow later).
- **Zero-copy when possible**: a Rust task feeding a Rust task on the same machine passes the value
  in memory without serializing it.
- Storage tiers:
  1. **Local memory** on the producing worker. The scheduler prefers to run consumers there.
  2. **Redis**: small values plus a directory of where each value lives.
  3. **S3-compatible object store**: large values, referenced by hash.
- Values are content-hashed, so workers can cache them and deduplicate transfers.

## 9. Tasks

- Language-neutral protocol: serialized inputs in, a serialized result or error out, plus task
  metadata (kind, resources, codec).
- Kinds: `single`, `batch` (the engine groups items across contexts, and flushes a partial batch
  after a timeout or when its scope ends), `stream`.
- Execution modes: **in-process** (Rust, fastest) and **out-of-process** (a warm process pool or
  sidecar, used for Python).
- Tasks may hold state (a loaded model, connections) via `prepare`, and are reused across calls.
- SDKs generate the protocol glue: Rust `#[task]`, and a Python SDK later.

## 10. Distribution

- **Control plane (orchestrator)**: stores workflows and tasks, starts runs, tracks membership and
  run status, and makes coarse placement decisions. It is **not** on the per-step path.
- **Data plane (workers)**: run the kernel, execute tasks, hold local values, and hand contexts
  to one another directly.
- Placement algorithm: to be designed. Inputs will include data locality, resource needs (GPU,
  memory), load, and work stealing.

## 11. Service API

API-first: publish workflow and task versions; start runs (by API call, schedule, or event);
query run status, events, and results; stream the results of unbounded runs.
Auth and multi-tenancy come later, but the API shapes should not preclude them.

## 12. Performance targets

- Same machine: framework overhead per step of **≤ ~10 µs**.
- Across machines: **about one network round trip** per handoff, and nothing more.
- No synchronous durable write on the common path (§7).

## 13. Open questions

1. Deployment: how task code reaches workers (on-demand artifacts, container images, prebuilt
   worker images), and what "fast deploy" means concretely.
2. Placement and scheduling algorithm (§10).
3. Syntax for reduce and window (collect is implicit via `Vec<T>` parameters), and how ambiguous
   cases such as `Vec<Vec<T>>` resolve.
4. Windowing semantics and backpressure for unbounded streams.
5. Type system: how Rust and Python type information is represented and checked in the IR.
6. The exact termination-detection protocol for fan-in across machines.

## 14. Gap vs. current implementation

"Today" reflects the distribution proof of concept (`poc.md`, 2026-10-18); ✅ marks rows it closed.

| Area | Today | Target |
|------|-------|--------|
| Parallelism | One call per op; a context waits on each call. Parallelism comes only from fan-out | Dataflow within a block |
| Value tree | ✅ Distributed workers use immutable nodes with scope pointers (Redis + per-worker cache). Lookups walk the parent chain; no GC. `SimpleEngine` still mutates in place; `DynamicContextManager` is unused | O(log depth) lookups, GC |
| Placement | Always appends to the leaf; loops grow the chain without bound | Liveness-based placement, longest-lived-first ordering |
| `compare_context` | Implemented, never called | Remove (re-add only if scheduling needs a tree order) |
| Dedup | Every op runs per leaf | Results placed at the deepest input's depth; computed once per ancestor |
| Fan-out / fan-in | ✅ Type-driven via `IntoArg` (fan-out, collect, nesting, dropped failures) in the distributed runtime. Not in `SimpleEngine`; an empty fan-out ends the branch instead of collecting `[]` | Plus reduce and window |
| `return` in a branch | Not supported | Drops that item (filter) |
| Phis | ✅ Applied in parallel (regression test `loop_phis_are_parallel_assignments`) | |
| Literals | ✅ JSON-encoded; string literals are `String` | Typed IR values |
| Values | JSON at every boundary in the distributed runtime; unchecked `Any` in `SimpleEngine` | Codec abstraction plus types, zero-copy locally |
| Errors in DSL | A failed branch is dropped and recorded in run failures | Results the DSL can inspect |
| Distributed path | ✅ Worker-local kernel; orchestrator is control plane only (Redis, no Postgres) | Direct worker-to-worker handoff with ancestors attached |
| Streams (distributed) | Stream task output is fanned out once complete | Pipelined per item |
| Batch | Partial batch hangs in `SimpleEngine`; batch tasks not supported by workers | Cross-context batching with flush |
| Task boundary | ✅ `namu-guest` generates the WASM component (WIT) and native C ABI v2; bodiless `#[task]` signatures in workflows | Typed WIT signatures with generated stubs; Python SDK |
| Runtimes | ✅ Per-task `wasm` or `native`, routed by runtime and platform. Trust tiers removed from enforcement (manifest field kept) | Python, containers, GPU |
| Durability | ✅ Work items commit atomically (exactly-once effects); expired leases are reclaimed and recomputed from the last committed context; poison items fail after 3 attempts; `checkpoint` tasks commit immediately (see `poc.md`) | Idempotency keys for tasks; Redis persistence |
