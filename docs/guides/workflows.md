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

## Supported control flow
- `if` / `if-else`
- `while` loops
- early `return`. Inside a fanned-out branch, a `return` before the collect drops that item: a
  filter.

```rust
let x = identity(xs);           // one branch per item
if less_than(x, 25) { return; } // drops the small ones
let y = double(x);
sum(y)                          // collects the rest
```

Not yet supported:
- `match`
- `break` / `continue`

## Outputs
Each `return` reached adds to the run's results. The run is `succeeded`, or `partial_failed` if any
branch failed.
