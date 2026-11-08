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

## Signatures
A task's own types say what it takes and gives. In a task crate they derive
`namu_guest::Describe` (types shared with a workflow crate can derive `namu::Data` instead, which
describes too):

```rust
#[derive(Serialize, Deserialize, namu_guest::Describe)]
pub struct Stats { words: usize, top_word: String }
```

`namu build` records the signature in the task's manifest, and a workflow is checked against the
signatures of the task versions it pins when it is published, so a call with the wrong types is
refused then, not when it runs. In process, `FnTasks` knows each task's from its Rust types, and
`LocalCluster` checks a workflow before it runs it.

What is compared is what crosses, MessagePack, so types match by shape, not by name: a workflow's
`Page` and a task's `Doc` match if the task's fields are among the page's (the task ignores the
rest, and a field it has that the page lacks must be an `Option`). Enum variants match by name,
and every variant a value may be must be one the reader has. Integers of any width match each
other, and a float reads an integer: a value that does not fit fails its call. Field and variant
names are serde's (`rename`, `rename_all`); a type whose shape serde changes in a way not modelled
(`flatten`, `tag`, `untagged`, `default` on a field that is not an `Option`, ...) is not checked.

## Errors
A task that fails with `anyhow::Error` (`anyhow::Result<T>`) fails with a message. One that fails
with a type of its own, which serializes, derives `Describe` and implements `Display`, gives that
value to a workflow that attempts the call, beside its message (its `Display`):

```rust
#[derive(Serialize, Deserialize, namu_guest::Describe)]
pub enum FetchError { NotFound, Moved(String), RateLimited { retry_after: u64 } }

namu_guest::task! {
    fn fetch(url: String) -> Result<Page, FetchError> { .. }
}
```

The workflow declares it the same way (`#[task] fn fetch(url: String) -> Result<Page, FetchError>;`)
and can match on its variants; `namu build` records the error type in the signature, and a
workflow that catches another is refused. A batch task's items fail the same way: a dynamic
one's (`Vec<Result<T, E>>`), and a continuous one's, whose `Continuous::Error` says what they fail
with (`anyhow::Error` for messages). So does a stream, opening or at an item, which ends it
(`Result<impl Iterator<Item = Result<T, E>>, E>`, one `E` for both). A typed attempt catches only
the error value: a trap or a panic fails the branch. In process, `FnTasks::raising("fetch", f)`
and `FnTasks::raising_stream` register tasks that fail with a type of their own.

A Python task declares it in its decorator, `@namu.task(error=FetchError)`, and raises
`namu.Failed(error)`; anything else it raises is a message. An `enum.Enum` crosses as its members'
names, so it reads as an enum of unit variants on the Rust side.

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

## Python tasks
Write the task with the `namu` package (`sdk/python`, no dependencies) and give its directory a
manifest with `"runtime": "python"`; the directory holds the one module:

```python
from dataclasses import dataclass
import namu

@dataclass
class Stats:
    words: int
    top_word: str

@namu.task
def summarize_py(stats: list[Stats]) -> dict[str, int]:
    return {"docs": len(stats), "words": sum(s.words for s in stats)}
```

The workflow declares it like any task (`#[task] pub fn summarize_py(stats: Vec<Stats>) -> ...`).
A worker that serves `python` (`NAMU_RUNTIMES=wasm,python`) runs it in a pool of Python processes
(`NAMU_PYTHON_PROCESSES`, default 4; `NAMU_PYTHON`, default `python3`), each running one call at a
time, awaited by the worker like an async task. Values cross as MessagePack: a struct is a map of
its fields, and a parameter annotated with a dataclass gets one. `namu.call_key()` gives the
call's key. The task runs in the worker's Python environment: install what it imports there. A
process that crashes fails its call and is replaced. See `examples/distributed/tasks/summarize_py`.

Streams and batches are written in Python too, and declared as such in the manifest
(`"task_kind": "stream"` or `"batch"`) and in the workflow (`#[task(stream)]`, `#[task(batch)]`).
An open stream, or a batch session, gets a Python process of its own for as long as it lasts, so
what the task loads once (a model, a connection) serves every item:

```python
@namu.stream
def lines(path: str, *, cursor: str | None = None):
    # Reopened after a crash, the stream is given the last cursor it yielded.
    with open(path) as f:
        f.seek(int(cursor or 0))
        while line := f.readline():
            yield namu.Item(line, cursor=str(f.tell()))

@namu.batch
def classify(texts: list[str]) -> list[str]:
    # One output per input, in order; an exception in an output's place fails that item alone.
    return model.predict(texts)

@namu.continuous
class Decoder:
    # A session is one instance. Each step takes in new (id, input) pairs and returns the
    # (id, output) pairs that are done, from this step or earlier ones.
    def step(self, admit: list[tuple[int, Prompt]]) -> list[tuple[int, str]]:
        ...
```

A stream that yields plain values (not `Item`s) has no cursor: reopened, it starts over and the
engine skips what it had yielded. `namu.call_keys()` gives the keys of a batch step's calls. See
`examples/distributed/tasks/{replay_py,classify_py,refine_py}` (workflow `label_stats_py`).

A Python task's signature is read from its annotations by parsing the module, so nothing it
imports needs to be installed where it is built: `int` is `i64`, `float` `f64`, `list[T]` a list,
`dict[K, V]` a map, `tuple[A, B]` a tuple, `T | None` an option, and a dataclass or `TypedDict`
defined in the module a struct of its fields. A stream is annotated `Iterator[T]` (or
`Iterator[namu.Item[T]]` with cursors), a batch task `list[T] -> list[R]`, and a continuous one by
its `step`. What is not annotated, or is imported from elsewhere, is not checked.
`python -m namu.signature module.py task` prints what `namu build` records.

## Async tasks
A task that mostly waits (on a model server, an API, another process) is registered as async:
the worker awaits it, so a hundred calls in flight hold no task slots and no threads.

```rust
let tasks = FnTasks::new()
    .with_async("complete", |prompt: String| async move { llm.complete(&prompt).await })
    .limit("complete", 32);   // at most 32 calls in flight per worker (default 256)
```

## Side effects: the call key
A task can run more than once for one call: after a crash, the engine runs again what the dead
worker had not finished. A task whose effect must happen once (a payment, an email, a write to
another system) passes its **call key** to that system as an idempotency key. The key is the same
each time the engine runs the call again, and different for every other call, loop iterations
included:

```rust
namu_guest::task! {
    fn charge(order: Order) -> anyhow::Result<Receipt> {
        let key = namu_guest::call_key().expect("called by the engine");
        payments().charge(&order, &key) // the payment service dedups on `key`
    }
}
```

In-process tasks read it with `namu_engine::call_key()`, async ones included (across awaits). A stream has the key of the call that
opened it, while it opens and while each item is pulled, however often it is reopened after a
crash. A batch step serves many calls, so it reads them all: `call_keys()` (in the guest or the
engine) lists the keys of its items, in the order of the list a dynamic batch task is given, or of
`admit` for a continuous one, which keeps a key with its item if it needs it later.
A task that must never run twice once it has returned is a checkpoint task instead (see
`docs/design.md` §7.5); the key covers a crash during the call too.

A task is otherwise a function of its inputs: a call that every branch of a fan-out makes with the
same inputs is made once, for all of them (`docs/design.md` §6.1). A checkpoint task is called by
every branch that calls it; a task whose result must differ from call to call takes something that
does (the item, or its call key).

## Retries
A task whose failures pass (a rate limit, a server restarting, a GPU out of memory once) gets a
retry policy rather than a loop in every workflow: `"retries": 3` in its manifest (with
`"retry_backoff_ms"`), or `.retry("fetch", Retry::new(3, Duration::from_millis(100)))` on
`FnTasks`. The engine tries the call again under the same key, backing off, and counts each in
`namu_task_retries_total`. A workflow that must decide what a failure means still matches on
`attempt(..)`, which sees the last try's error.

A failure with only a message (a crash, or a task that fails with messages) is tried again. A
task's own error value is its answer, and is not: `NotFound` will not pass. The variants that do
pass are named, as serde names them: `"retry_on": ["rate_limited"]` in the manifest, or
`Retry::new(3, backoff).on(["rate_limited"])`.

## Task kinds
- `single`: one input, one output
- `batch`: written for a list of inputs, called per item; a session on each worker takes items
  from every branch and run, step by step
- `stream`: one input, outputs yielded over time; a list in the workflow, fanned out as it
  arrives
- `reduce`: `(accumulator, item) -> accumulator`, called with a first accumulator and a
  per-item value; folds the items of a fan-out in order. It is an ordinary two-argument task to
  the engine and the runtimes: only the workflow declares it `#[task(reduce)]`
