# Tasks

Tasks are the smallest unit of execution. NAMU supports two usage modes: in-process execution (for local engines and examples) and artifact-based execution (for workers).

## In-process tasks (SimpleEngine)
Use the `#[task]` macro and register the task so the engine can call it.

```rust
use namu::prelude::*;

#[task(single)]
pub fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}

register_task! {
    method = add,
    name = "add",
    author = "Namu",
    version = "0.1.0"
}
```

This pattern is used in `examples/advanced` and is ideal for local testing or embedded use.

## Worker-executed tasks (artifacts)
Workers load task artifacts from `namu build` and expect C ABI exports:
- `namu_task_create`
- `namu_task_destroy`
- `namu_task_call`

See `tests/e2e/tasks/add` for a minimal implementation. For native tasks, build a `cdylib`. For WASM tasks, compile to `wasm32-wasip1` and export the same symbols.

## Task kinds
- `single`: one input tuple, one output
- `batch`: vectorized inputs, vectorized outputs
- `stream`: iterator-style input/output

The `task_kind` must match the implementation and the manifest.
