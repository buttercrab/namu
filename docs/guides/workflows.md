# Workflows

Workflows are Rust functions annotated with `#[workflow]`. The macro rewrites the function into an SSA graph and emits JSON IR at compile time.

## Example
```rust
use namu::prelude::*;

#[task(single)]
fn add(a: i32, b: i32) -> Result<i32> { Ok(a + b) }

#[workflow]
fn demo() -> i32 {
    let x = add(1, 2);
    let y = add(x, 40);
    y
}
```

## Inputs
A workflow's parameters are its inputs: what a run gives it, so one published version serves any
number of runs.

```rust
#[workflow]
fn index(limit: i32, collection: String) -> IndexReport {
    let article = articles(limit);
    // ...
}
```

A parameter is a plain name with a type (`mut` if loops reassign it). Runs give each by name:

```bash
namu run index 0.1.0 --input limit=300 --input collection=wikipedia --wait
```

```rust
let inputs = namu::Inputs::new().with("limit", 300).with("collection", "wikipedia".to_string());
cluster.run_with(index().to_serializable("index".into()), inputs).await?;
```

Inputs are checked before the run starts: a missing one, one the workflow does not have, or one of
the wrong type is refused with what is wrong, e.g. ``invalid inputs: `limit`: expected i32, got a
string``. Through the API, inputs are JSON, written as serde writes the type: a struct as an
object (a missing `Option` field is `None`), a newtype as what it wraps, an enum's unit variant as
its name and any other as `{"Variant": ..}`. Map inputs need string keys.

## Types
The values passed between tasks are `namu::Data`: the standard types are, and your own derive it,
which also gives the IR their shape:

```rust
#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Chunk { pub doc_id: u64, pub text: String }
```

Every value in the IR records its type, so a UI or a task in another language knows what flows
where.

## Workflow ids
By default the workflow id is the function name. You can override it:

```rust
#[workflow(id = "etl_pipeline")]
fn pipeline() -> i32 { /* ... */ }
```

## Fan-out and fan-in
Parallelism comes from types, not loops:

```rust
let docs = split_docs(48);       // Vec<String>
let stats = analyze(docs, cfg);  // analyze(doc: String, ...): one branch per document
summarize(stats)                 // summarize(stats: Vec<Stats>): collect every branch
```

- A `Vec<T>` passed where a task takes `T` fans out. Everything after it runs once per item.
- A per-item value passed where a task takes `Vec<T>` collects the surviving branches.
- Values from before the fan-out (like `cfg`) are shared by every branch.
- A call every branch makes from values from before the fan-out is made once, for all of them,
  as if it had been written before it: `load_model(path)` after `analyze(docs, cfg)` loads the
  model once. Not one only some branches make (behind an `if`), nor a checkpoint or stream
  task's.
- A failed branch is dropped from the collect, and the run ends `partial_failed`.

To fold the items instead of collecting them all, call a reduce task with a first accumulator
made before the fan-out:

```rust
#[task(reduce)]
pub fn merge_stats(summary: Summary, stats: Stats) -> Result<Summary>;

let empty = empty_summary();          // before the fan-out
let stats = analyze(docs, cfg);       // per document
merge_stats(empty, stats)             // folds every document, in order, as they finish
```

Items are folded in document order whatever order they finish in, so the merge need not be
commutative, and only items that finish early wait in memory.

`each(list)` fans out without a task, and `window(item, n)` groups the items again, `n` at a
time, in order, each group going on as a branch of its own; over a stream that never ends, the
windows keep coming:

```rust
let doc = each(stream_docs(n, delay));   // one branch per document, as it arrives
let stats = analyze(doc, cfg);
let batch = window(stats, 8);            // a Vec<Stats> for every 8 documents
let summary = summarize(batch);          // once per window
total_words(summary)                     // collects the windows
```

## Expressions
Arithmetic, comparisons and logic on values are built-ins: the engine evaluates them itself,
with no task call.

```rust
let mut tries = 0;
while tries < 3 && !done {
    tries += 1;
}
let score = hits * 100 / total;
let label = exclaim(name) + "?";   // + on strings concatenates
```

- Operators: `+ - * / %`, unary `-`, `== != < <= > >=`, `&& || !`, and `+=` and the like.
- Types: `bool`, `i32`, `i64`, `u32`, `u64`, `f32`, `f64`, `String`. Both sides of an operator
  have one type; rustc checks it.
- Integer arithmetic is checked: an overflow or a division by zero fails the branch.

Values are taken apart and put together the same way:

```rust
let page = fetch(url);
let first = page.links[0];               // a field, then an element
let (lo, hi) = (page.span.0, page.span.1);
let row = (page.url, page.words * 2);    // a tuple
let line = format!("{} has {} words", page.url, page.words);
```

- Field access on structs (`page.url`) and tuples (`pair.0`), and indexing lists (`xs[i]`, with
  `i` a `usize`): rustc checks the access as it would in Rust. An index out of bounds fails the
  branch. The engine finds the part in the value's bytes, without decoding the rest.
- Tuples of up to 8 values.
- `format!` with `{}` and `{name}` placeholders, each a scalar (one of the types above), written
  as `Display` writes it. Formatting options (`{:.2}`, `{:>8}`) are a task's to do.

## Supported control flow
- `if` / `if-else`
- `while` loops, with `break` and `continue` (unlabeled; on the innermost loop)
- early `return`. Inside a fanned-out branch, a `return` before the collect drops that item: a
  filter.

```rust
let x = identity(xs);           // one branch per item
if x < 25 { return; }           // drops the small ones
let y = double(x);
sum(y)                          // collects the rest
```

- `match attempt(task(..))`: errors as values (below), and `match` on an enum's variants

Not yet supported:
- `match` on anything but an attempt or an enum (an `Option`, a number)
- labeled `break` / `continue`, and `break` with a value

## Errors as values
A task that fails fails its branch. To handle the failure instead, attempt the call and match on
it:

```rust
let x = identity(xs);
let y = match attempt(fetch(x)) {
    Ok(page) => page,
    Err(message) => fallback(x, message), // `message` is the task's error, a String
};
```

`Ok` binds the call's outputs (a tuple pattern for a task with several), `Err` its error;
either may be `_`. A task declared with an error type of its own,
`fn fetch(url: String) -> Result<Page, FetchError>`, gives that value instead of its message, and
the `Err` arms can match on its variants:

```rust
let page = match attempt(fetch(url)) {
    Ok(page) => page,
    Err(FetchError::NotFound) => placeholder(url),
    Err(FetchError::Moved(to)) => fetch_again(to),
    Err(FetchError::RateLimited { retry_after, .. }) => later(url, retry_after),
};
```

A typed attempt catches only the task's own error. A failure with nothing of that type to give (a
trap, a panic, a crashed process) fails the branch, as an unattempted failure does. Both arms are needed, and they join afterwards like the arms of an `if`, so a
variable assigned in one arm has the right value after the `match`. Retries are a loop:

```rust
let mut tries = 0;
let mut out = 0;
while less_than(tries, 3) {
    match attempt(flaky(x)) {
        Ok(v) => { out = v; tries = 3; }
        Err(_) => { tries = add(tries, 1); }
    }
}
```

An attempt only catches the task's own failure. Crashes and lost workers are not errors a workflow
sees: the engine recovers from them.

## Matching on enums
`match` works on any enum value, a task's result as well as an error, if the enum derives
`namu::Data` (or `Describe`) and keeps to serde's default form:

```rust
match classify(doc) {
    Kind::Spam => discard(doc),
    Kind::Topic(name) => file_under(doc, name),
    Kind::Mixed { topics, .. } => split(doc, topics),
}
```

An arm is a variant, with its fields bound to names or left out (`_`, `..`), `_`, or a name for
the value; rustc checks that the arms are the enum's and cover it. Variants are written with their
path (`Kind::Spam`), or by a name that starts with a capital. Guards, `|` and patterns inside
fields are not supported.

## Outputs
Each `return` reached adds to the run's results. The run is `succeeded`, or `partial_failed` if any
branch failed (a caught error does not count).
