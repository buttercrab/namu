# Tasks

Tasks are the smallest unit of execution. A workflow only declares a task's signature; the task
itself is implemented once, for the runtime that runs it.

## Tasks for workers (`namu-guest`)
Write the task once and build it for the runtime its manifest declares:

```rust
// Cargo.toml: crate-type = ["cdylib"], depends on namu-guest
namu_guest::task! {
    fn analyze(doc: String, min_len: i32) -> anyhow::Result<Stats> {
        Ok(Stats::of(&doc, min_len))
    }
}
```

- `runtime = "wasm"`: built with `--target wasm32-wasip2` into a WASM component. One artifact runs on
  every platform.
- `runtime = "native"`: built as a native library for the platform it is built on.

Inputs and outputs are MessagePack. With no arguments the input is nil, with one it is the value
itself, and with several it is an array; errors are text. One task per crate. See
`examples/distributed/tasks`.

The workflow crate declares the task's signature without a body, so it never links task code:

```rust
#[task]
pub fn analyze(doc: String, min_len: i32) -> Result<Stats>;
```

## Stream tasks
A stream task yields its outputs one at a time, like a crawler or a reader over a large file:

```rust
namu_guest::stream! {
    fn pages(site: String) -> anyhow::Result<impl Iterator<Item = anyhow::Result<Page>>> {
        Ok(crawl(site))
    }
}
```

Its manifest says `"task_kind": "stream"` (and optionally `"window"`, below). The workflow declares
it with `#[task(stream)]`, and its output is a list:

```rust
#[task(stream)]
pub fn pages(site: String) -> Result<impl Iterator<Item = Result<Page>>>;

let pages = pages(site);       // Vec<Page>
let parsed = parse(pages);     // parse(Page): one branch per page, as each arrives
```

When the stream's only use is a fan-out, each item becomes a branch as soon as it is yielded,
while the rest are still coming. At most `window` items (default 1024) are in flight at once: the
stream is not asked for more until branches end, so a slow consumer slows the producer. If the
worker running the stream dies, its successor resumes the stream where it was (design §6.5). A
stream used whole (passed as a list) is read to its end first.

WASM stream tasks implement the `stream-task` world; native ones export `namu_stream_open`,
`namu_stream_next` and `namu_stream_close` (C ABI version 3). `namu_guest::stream!` generates both.

## Batch tasks
A batch task is written for many items at once but called per item; the runtime groups items
from every branch, and every run, into steps of a long-lived **session** on a worker (holding a
loaded model, say). Each branch gets its own result as soon as it is ready (design §6.6).

```rust
// Dynamic batching: each step runs the function over the items it is given.
namu_guest::batch! {
    fn embed(texts: Vec<String>) -> Vec<anyhow::Result<Embedding>> {
        model().embed(&texts)
    }
}

// Continuous batching: items stay in the session across steps, and each leaves when it is done.
#[derive(Default)]
struct Decoder { /* active sequences, KV cache, ... */ }

impl namu_guest::Continuous for Decoder {
    type In = Prompt;
    type Out = Text;
    fn step(&mut self, admit: Vec<(u64, Prompt)>) -> Vec<(u64, Result<Text, String>)> {
        // admit new prompts, run one decoding step, return the finished ones
    }
}
namu_guest::continuous!(Decoder);
```

The workflow declares either as `#[task(batch)] pub fn embed(texts: Vec<String>) ->
Vec<Result<Embedding>>;` and calls it per item: `embed(text)` (or with a list, which fans out).
The manifest says `"task_kind": "batch"`, with optional `batch_size` (items in a session at once,
default 32), `max_wait_ms` (wait for a full batch; by default a session steps as soon as it is
idle) and `batch_slots` (sessions per worker). WASM batch tasks implement the `batch-task` world;
native ones export `namu_batch_open`, `namu_batch_step` and `namu_batch_close` (C ABI version 5).

## In-process tasks (`FnTasks`)
In the same process as the engine, a task is a Rust function registered by name. Its input is the
argument itself for one, a tuple for several, `()` for none. Values pass between in-process tasks
as they are, never encoded, so their types implement `namu::Data`: the standard types do, and your
own derive it:

```rust
#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Label { text: String, label: String }
```

```rust
let tasks = FnTasks::new()
    .with("add", |(a, b): (i32, i32)| Ok::<_, String>(a + b))
    .with_stream("count_to", |n: i32| Ok::<_, String>((0..n).map(Ok::<_, String>)))
    .window("count_to", 16)
    .with_batch("embed", |texts: Vec<String>| Ok::<_, String>(embed_all(texts)))
    .with_continuous("decode", Decoder::default);
```

See `examples/basic`.

## Task kinds
- `single`: one input, one output
- `batch`: written for a list of inputs, called per item; a session on each worker takes items
  from every branch and run, step by step
- `stream`: one input, outputs yielded over time; a list in the workflow, fanned out as it
  arrives
