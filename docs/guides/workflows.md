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

## Supported control flow
- `if` / `if-else`
- `while` loops

Not yet supported:
- `for` loops
- `match`
- `break` / `continue`
- tuple destructuring beyond arity 2

## Outputs
Workflow outputs are serialized and stored as run values. The orchestrator marks the run as `succeeded` or `partial_failed` depending on leaf failures.
