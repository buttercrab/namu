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

Built-ins are evaluated where the branch stands, in the step that applies the operation's phis,
with no task call: a loop of arithmetic and comparisons never leaves its worker. Rust types them:
`a + b` lowers to a generic helper over the operands' type, so `String - String` or `i32 < String`
is a compile error, and the IR records the type, so the engine reads the operands exactly.
Integer arithmetic is checked; an overflow or a division by zero fails the branch with a message.

### 4.4 Errors

A task call that fails ends its branch only; sibling branches continue. A workflow that wants to
handle the failure attempts the call: `match attempt(task(x)) { Ok(v) => .., Err(e) => .. }`,
where `e` is the task's error message. Retries, fallbacks and dead-letter handling are written in
the DSL with it.

In the IR an attempted call has an **error edge** (LLVM's `invoke`): on failure its outputs are
not defined, the error is bound instead, and control goes to the handler. So an error is an
ordinary value on an ordinary edge: liveness, scopes and recovery see one more successor, not a
new kind of value, and a call that is not attempted costs nothing extra. An attempted call does
not run in a parallel group or feed a fused stream, since where control goes depends on how it
ends.

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
         C0   C1   C2     one child per item, numbered 0..n
         |    |
        ...  (C1 fans out again: a new scope, nested in S)
         |
      reaches S's collect -> sends its index and item to S's owner
```

- Which collect closes which fan-out is fixed by the control-flow graph, so it is found from the
  IR before the run. A scope with no items, or whose items were all dropped, therefore still
  collects `[]` and continues.
- When the scope is over, the collected list is written into a **new child of F**, and the branch
  that fanned out continues from there. Downstream code sees everything from before the fan-out
  plus the collected value, but not per-item values. A scope without a collect simply ends the
  branch that opened it.
- Knowing when a scope is over across machines is done by **counting**. Each scope has an
  **owner** (the worker that opened it, encoded in the scope's id), and every branch of the scope
  is created by its owner: one per item when it fans out, or one per item its stream source
  yields. So the owner knows the scope's **count**: at once for a fan-out, and when the source
  ends for a stream. Branches are numbered 0..count, a branch that ends sends its index to the
  owner (with its collect item if it has one), and the scope is over once every index has ended.
  No other worker creates branches of the scope, so no message ordering between workers can end
  it early, and no global lock is needed.
- The owner keeps the ended indices as a low watermark plus those that ended ahead of an earlier
  one, so its memory is the out-of-order window, not the number of branches. The same set drops a
  branch that ends twice after recovery.
- A branch that reports a result or a failure to the run's owner ends in its scope only once the
  owner has recorded the report. So a run cannot finish while a report is still in flight.
- Ends can be batched per worker. For reduce, each worker folds locally and sends one partial
  result with the indices it covers.

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

A **stream task** yields items over time; it does not return a list. In a workflow its output is
a list (`#[task(stream)] fn crawl(seed: Url) -> ...` has type `Vec<Page>`), so it fans out and
collects like any list. When the stream's only use is the fan-out right after it, which the
engine finds in the IR, the two are fused: each item becomes a child context of the fan-out scope
**as soon as it is yielded**. Streams are then pipelined: item 1 can be at stage 3 while item 50
is at stage 1. A stream used any other way is read to its end into a list.

- **The source owns the scope.** The worker that runs the stream task opens the scope and keeps
  the source going. Item k is branch k of the scope; when the stream ends, its count is known. A
  stream that fails reports the failure as one more branch.
- **Backpressure, with credits.** The source yields an item only while fewer than `window` of the
  scope's branches are open, and a branch that ends frees its credit. Since the source and the
  scope live on the same worker, credits never cross the network: a slow GPU downstream slows the
  crawler, instead of filling memory, and no wait can form a cycle.
- **Resumable sources.** A source that runs for weeks must survive a crash without starting over.
  Its position (items yielded, and the last cursor if the task yields cursors: an offset, a page
  token) is logged with the scope. A successor reopens the stream at the cursor and skips to the
  position, inside the same scope. Items are numbered, so an item yielded again has the same
  branch id and counts once.
- **Bounded vs unbounded.** A bounded stream ends and its scope can be collected. An unbounded
  stream never ends, so it can only be consumed per item, or through a window (§4.5).

### 6.6 Batching

A **batch task** is written for many items at once (`fn label(xs: Vec<Item>) -> Vec<Label>`) but
is called per item in the workflow (`label(item)`), like a single task; passing it a list fans
out. Batching is a runtime concern, not a workflow construct: each branch keeps its own context,
waits for its own result, and continues independently. It is **not** a fan-in.

A batch task runs as a **session**: a long-lived instance on a worker (holding a loaded model,
say), driven in **steps**:

```
open()                               -> session
step(session, admit: [(id, input)])  -> [(id, result)]
```

Each step hands the session newly arrived items and takes back the items it has finished, in
any order. Every result resumes its branch at once. One call does both, so no threads cross the
task boundary, and it works the same in WASM, native code and in-process.

- **Dynamic batching** is the simplest session: a step runs the task's `fn(Vec<T>) ->
  Vec<Result<R>>` over the items it is given and returns them all. An idle session takes whatever
  is queued (up to `batch_size`); items that arrive while it is busy form the next step. Under
  light load an item runs at once; under heavy load batches fill up by themselves.
- **Continuous batching** (as in LLM serving) is a session that keeps items across steps: one step
  is, say, one decode iteration over the active sequences. A finished item leaves at once and a
  waiting one takes its place, so no item waits for the slowest in its batch.
- **Capacity**: at most `batch_size` items are in a session at once. Optionally, `max_wait` holds
  a step until that many are queued or the oldest has waited that long, for fuller batches.
- **Across runs**: sessions belong to a worker and a task version, not to a run. Items from
  different scopes and different runs of the same task version share steps. This is how one
  inference worker serves many concurrent labeling runs.
- A worker runs one session per batch task (`batch_slots` for more, e.g. one per GPU). Items go to
  the least loaded worker that serves the task (§10.3); under dynamic batching each one's batches
  fill under load. Filling one worker's batch before the next also needs to know how full each
  worker's batch is, which load alone does not say.
- A failed step fails every item in it; an item's own error fails only its branch. A worker that
  dies mid-step loses nothing durable: its waiting and in-flight branches run again from their
  records (§7), like any task call.

This is the stream machinery of §6.5 in reverse: a stream source is a long-lived instance that
produces items into branches; a batch session consumes branches and produces their results, as
they finish.

## 7. Durability

Durability comes from **lineage**: every value is the output of a known task applied to known
immutable inputs. A crash loses work in progress, never the run: what was lost is recomputed from
the last durable point, and duplicates of recomputed work are recognized and dropped.

### 7.1 Identity

- Every **branch** has a stable id: the run's root branch, or (scope, item index) for a fan-out
  child. A branch keeps its id across handoffs, and the branch that opened a scope keeps its id
  when it continues after the collect.
- A branch's **hop** counts its handoffs, so each stop of a branch on a worker is a distinct
  record.
- **Scope owners drop a second `End` from the same branch, and run owners a second `Report`.**
  So if recovery executes a branch again, it still counts once: effects on NAMU state are
  exactly-once, while task execution is at-least-once.

### 7.2 What is logged

The **durable log** (a trait; Redis with persistence first) holds just enough to resume every
live branch and every open scope. Writes are behind the critical path: nothing waits for them
except the acknowledgement that releases the sender's copy.

| Record | Written by | When |
|---|---|---|
| Run description | the run's owner | when the run starts |
| Branch (context, resume point, scopes) | the worker holding it | when the branch arrives there |
| Scope (the branch that opened it, its collect, its count if known) | the scope's owner | when it fans out; a stream's count when the stream ends |
| End (branch index, collected item) | the scope's owner | when a branch ends in the scope |
| Report (branch id, result or failure) | the run's owner | when a branch reports |
| Nodes | whoever writes a record that refers to them | with that record |

**Nodes are logged only when something durable refers to them.** A record refers to a context.
The nodes it needs are the ones created since the branch arrived on this worker, and they are
logged with the record. These are exactly the nodes a handoff ships to the receiver, and older
ancestors were logged when the branch arrived. So everything a live branch can reach is durable,
without a log write per step.

### 7.3 Upstream backup

Messages that move responsibility for a branch (`Handoff`, `End`, `Report`) are kept by their
sender until the receiver acknowledges that the record they produce is durable. The sender then
deletes its own record of the branch: at every moment, a live branch is recorded on its holder
or still held by its sender. A fan-out releases the opening branch's record only once the scope
record is durable and every child has been acknowledged. Acknowledgements are batched and
asynchronous; the receiver starts on the work immediately.

### 7.4 Recovery

- When a worker's membership expires (§10.4), the orchestrator names **one successor** for it,
  and every member learns this from its next heartbeat. Exactly one worker adopts, because a scope
  must have a single owner to count its branches.
- Messages addressed to the dead worker go to its successor from then on, and senders re-send
  everything the dead worker had not acknowledged. Nodes it held are read from the log.
- The successor **adopts the dead worker's estate** from the log:
  - its runs, with their reports;
  - its scopes, with their ends;
  - the branches it held. Those branches run again from their recorded point.
- Work that ran twice is dropped by branch id (§7.1). A scope whose opening branch is run again is
  opened again under a new id, and branches that end in the abandoned scope are dropped. The
  result is the same as an uninterrupted run; only work done since the last durable point
  is repeated.
- If the successor dies too, its successor adopts both estates.
- Recovery is driven by events, never by timers. A message for a dead worker that arrives before
  the successor has adopted its estate waits for the adoption. A message sent to a worker that has
  not yet heard of a death waits for its next membership update. A run's owner waits for every
  live worker to confirm the end of the run, and stops waiting for a worker when it leaves the
  membership.

### 7.5 Checkpoints and side effects

- **Checkpoint tasks**: tasks with external side effects (writes to DBs or APIs) are marked as
  checkpoints. After one runs, its branch is recorded again, past the task, and continues only
  once that record is durable. Recovery therefore never runs it again (a crash during the task
  itself still repeats it).
- Task execution is **at-least-once**. Each call gets a deterministic **idempotency key** so
  side-effecting tasks can deduplicate: the run id, the branch's **lineage** and **clock**, and
  the op. Both are logical, never minted while running. The clock counts the branch's calls and
  fan-outs; it travels and is logged with the branch, so a branch run again from a record counts
  exactly as it did the first time, and the calls of a loop each get their own key. The lineage
  is a hash of the branch's path through the fan-outs (its parent's lineage, the parent's clock
  at the fan-out, the item's index), so a fan-out done again after a crash (its scope not yet
  durable, so its scope and branch ids are minted anew) names its items' calls as before. Tasks
  read it in the guest
  (`namu_guest::call_key()`, passed through the task ABI, version 6) or in process; a stream has
  its call's key, recorded with its source, for as long as it runs and whenever it is reopened;
  a batch step has the keys of its items (`call_keys()`).
- **Long runs stay small.** With liveness-based placement (§6.2), dead nodes are freed, in the log
  as well as in memory. The persisted state is then the live frontier of the run, not its history,
  so a run's footprint and its recovery time do not grow with its age.

## 8. Values and storage

### 8.1 The value

A value is the user's erased `Value` (`namu_core::value`): any Rust value, behind one pointer to
a static vtable for its type.

```
Value { payload: two words, vtable: &'static VTable }

payload   inline   T fits in two words (i32, f64, bool, small enums): stored in place
          shared   anything larger: an Arc<T>, so cloning a value is a reference count
VTable    type_id, drop, clone, encode (to MessagePack), and the shape of T (§8.2)
```

- **Cloning is cheap.** The value tree hands one value to many branches (every read of a shared
  ancestor), so a clone never copies the data: inline values are copied as words, shared ones
  bump a count. `take::<T>()` moves `T` out when this is the last reference, and clones it
  otherwise.
- **Values are typed until they leave the process.** A Rust task's output stays a live `T`; the
  next Rust task on this worker gets `&T` (or `T`) back without any encoding. This is what makes
  a workflow on one machine cost about what calling the functions directly costs.
- **Encoding happens at a process boundary, once.** A value that goes to another worker, into the
  log, or into a WASM or native task is encoded to **MessagePack** through its vtable. A value
  that arrives encoded is an `Encoded` value (its bytes, shared): it is decoded into `T` only when
  a Rust task asks for a `T`, and the decoded value is kept, so it is decoded once. A WASM task, a
  further hop or the log gets the bytes as they are.

### 8.2 Shapes: what the engine looks inside

The engine reads values in exactly three places: a fan-out splits a list, a collect builds one,
and a branch reads a bool. Everything else passes through untouched. So a type tells the engine
its **shape**:

| Shape | Types | Fan-out | Collect |
|---|---|---|---|
| list | `Vec<T>`, tuples, encoded arrays, collected lists | items: a clone of each element of a `Vec<T>`; slices of an encoded array; the values of a collected list | a list of the item values, whatever their types; it becomes a `Vec<T>` when something asks for one |
| bool | `bool` | | |
| opaque | everything else | | |

- An encoded list is split by reading the MessagePack array header: each item is a slice of the
  same bytes, so nothing is copied or decoded.
- A collect builds a list of values without knowing their type, so building it never needs a
  `Vec<Vec<…>>` vtable (which a type-built list would, without end). Asking it for a `Vec<T>`
  takes each item as a `T`.
- Rust has no specialization, so a vtable cannot discover on its own that some `T` is a
  `Vec<U>`. The shape therefore comes from a trait, **`Data`**, implemented for the primitives,
  `String`, `Vec<T: Data>`, `Option<T: Data>` and tuples, and derived for user types with
  `#[derive(namu::Data)]` (opaque, alongside `Serialize`, `Deserialize` and `Clone`). The DSL
  already requires these types to be serializable; `Data` is the one addition.

### 8.3 Where values come from and go

- **Literals** stay JSON text in the IR, so the IR stays readable to people and to the tools that
  will write it. They are decoded once per run (`kernel::Literals`), not on every op entry.
- **Rust tasks in process** (`FnTasks`): a task takes its input by type (the argument itself, a
  tuple for several, `()` for none) and its output is wrapped as a value. A task with several
  outputs returns a tuple, which splits into them.
- **WASM and native tasks** (task ABI v6): the arguments cross as one MessagePack document (nil,
  the argument, or an array), outputs as MessagePack, errors as text. The WIT types are
  `list<u8>`; `namu-guest` does the encoding. Outputs stay encoded until a Rust task asks for a
  type.
- **Wire and log**: mesh frames and log records are MessagePack. A node is encoded the first time
  it crosses a boundary and keeps its encoding, so a node shipped to several workers and logged is
  encoded once; a node that arrived encoded goes on as the bytes it came as.
- **The service API** (HTTP) shows results as JSON, converted from MessagePack at that edge only.
  Values are JSON nowhere else.

### 8.4 Storage

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
- **Tasks are CPU-bound by contract, and run where the branch is driven.** A call runs on the
  driving thread (`block_in_place` on a multi-threaded runtime), bounded by the worker's task
  slots: no hop to a blocking pool and back, which on the per-step path costs more than a small
  task does. A task that mostly waits (on the network, on a subprocess) holds a thread while it
  waits, so those get their own kind later: an **async task** that the worker polls instead of
  blocking on, which is also how out-of-process runtimes (a Python pool, a sidecar) will be
  called.

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
Handoff { id, run, resume_at: op, parent: node ref, values: inline live values }
```

The receiver creates a node under `parent` holding `values`, and continues the kernel at
`resume_at`. Handoffs between a pair of workers share one connection and are batched when they
queue up. The transport is a trait: TCP or QUIC between machines, and channels between workers in
one process (which is also how a workflow runs locally and in tests: one engine, no special
local mode).

The transport delivers each message exactly once and in order between two live workers. Over TCP,
messages to a peer form a session: each carries a sequence number and is kept until the peer
acknowledges it, so a broken connection is reconnected and replayed, and the receiver delivers
each number once. Losing a peer is not the transport's concern: that is recovery (§7), which
re-sends to its successor what the dead worker had not made durable.

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
  no longer a member, the orchestrator names its successor, and recovery (§7.4) starts. A worker
  that comes back after being declared dead rejoins as a new member.

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

"Today" reflects `namu-engine`, which runs in-process (`LocalCluster`) and across machines
(`namu-worker` on the TCP mesh, see `poc.md`). ✅ marks rows that are done.

| Area | Today | Target |
|------|-------|--------|
| Parallelism | ✅ Fan-out, and a straight run of calls in one branch as a dataflow DAG: a worker serving them all starts each call once the calls it reads from have finished, stores their outputs in order, and reports the first failure in order, as one call at a time would (`namu_core::parallel`) | Across literals and phis in a block; calls of a group served by different workers |
| Value tree | ✅ Immutable nodes that own their parent: a context keeps its chain alive and nothing else, so a node is freed when its last child or context goes. A lookup walks up to 32 parents, then jumps in O(log depth): binary-lifting ancestors plus a persistent segment tree from value id to depth, built for a chain on its first deep lookup and kept. Workers index nodes weakly to serve fetches; a freed node asked for again is read from the log | |
| Placement | ✅ A node attaches under the deepest node still holding a live value (liveness from the IR; a node's own values shadow older copies, which is what lets loops stay shallow). Chains stay as deep as the values in use | Several nodes per step, ordered longest-lived first |
| Dedup | ✅ Constants are hoisted into the root context. Every other op runs per leaf | Results placed at the deepest input's depth; computed once per ancestor (§13 q7) |
| Fan-out / fan-in | ✅ Type-driven via `IntoArg`; collect ops found statically, so an empty or fully dropped scope collects `[]` (engine) | Plus reduce and window |
| `return` in a branch | ✅ Ends the branch; before a collect it drops the item (filter) | |
| Phis | ✅ Applied in parallel (regression test `loop_phis_are_parallel_assignments`) | |
| Literals | ✅ JSON text in the IR, decoded once per run; string literals are `String` | |
| Values | ✅ The erased `Value` (§8): typed and shared in process, MessagePack at process boundaries; `Data` gives the engine a type's shape | An object store for large values |
| Expressions | ✅ Built-ins: `+ - * / %`, unary `-`, comparisons, `&& || !` and compound assignment on `bool`, integers, floats and strings (`+` concatenates), typed by rustc and evaluated in the kernel; checked integer arithmetic (§4.3) | Field and index access, tuples, string formatting |
| Errors in DSL | ✅ `match attempt(task(..)) { Ok(..) => .., Err(e) => .. }`: the call gets an error edge in the IR (§4.4); a failure nobody attempts drops the branch and is recorded in run failures | Typed errors: a task's own error type, not only its message |
| Engine | ✅ One engine: workers, handoffs, scope owners, `Transport` trait. Every test runs on one worker, three in-process, and three over TCP | |
| Distributed path | ✅ Peer-to-peer TCP mesh with a reliable session per pair of workers (sequence numbers, replay on reconnect); the orchestrator is peer 0 for run start and end only; a handoff carries the nodes the receiver needs (~50 µs locally) | Binary codec; QUIC (§10.2) |
| Routing | ✅ To the least loaded worker that can take it, per task slot: each worker reports the branches it drives on the acknowledgements it sends, and a sender counts what it sent since. A worker that has not reported counts as idle; a branch waits for a server to join | Locality (holders of a context's nodes); packing batch items into one worker's batch; work stealing (§10.3) |
| Streams | ✅ Fused with their fan-out, per-item credits at the source, resumable by position (§6.5). Guest streams yield no cursors yet; a resumed stream is reopened and skipped forward | Cursors from guest streams; windows for unbounded streams |
| Batch | ✅ Sessions driven in steps: dynamic and continuous batching across branches and runs, `batch_size`, `max_wait`, `batch_slots` (§6.6) | Load-aware routing to fill one worker's batches first; several outputs per item |
| Task boundary | ✅ `namu-guest` generates the WASM component (WIT) and native C ABI v6; `#[task]` declares signatures only; `FnTasks` for in-process Rust | Typed WIT signatures with generated stubs; Python SDK |
| Runtimes | ✅ Per-task `wasm` or `native`, routed by runtime and platform | Python, containers, GPU |
| Durability | ✅ Stable branch ids with owner-side dedup, upstream backup, write-behind log (Redis; in memory for `LocalCluster`), successors adopt dead workers' estates, checkpoint tasks, a call key per call for tasks with side effects (§7). The log holds a run's live frontier: a scope's ends, count and position go with it, and nodes no live record reaches are swept whenever a run's nodes double (§7.5) | Reports, which are the run's results, stay until it ends |
| Fan-in termination | ✅ Counting at scope owners: every branch of a scope is created by its owner, so it knows the count; ended indices are a watermark plus the out-of-order window | |
| Membership | ✅ Join (register + heartbeats); a worker whose heartbeats stop is replaced by a successor (§7.4). Drain on SIGTERM (§10.4): routed around, branches move on at their next task call, stream sources stop at an item, and once nothing is in flight and everything is durable the worker leaves and its successor takes over its scopes from the log, with nothing run twice | Heartbeats tied to the registering process, not just its id |
