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
- Not yet: field and index access, tuples built from values, `format!`. Use a task.

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

- `match attempt(task(..))`: errors as values (below)

Not yet supported:
- `match` on anything but an attempt
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

`Ok` binds the call's outputs (a tuple pattern for a task with several), `Err` its error message;
either may be `_`. Both arms are needed, and they join afterwards like the arms of an `if`, so a
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

## Outputs
Each `return` reached adds to the run's results. The run is `succeeded`, or `partial_failed` if any
branch failed (a caught error does not count).
