# Tasks

Tasks are the smallest unit of execution. There are two ways to write them.

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

Inputs and outputs are JSON. With no arguments the input is `null`, with one it is the value
itself, and with several it is an array. One task per crate. See `examples/distributed/tasks`.

The workflow crate declares the task's signature without a body, so it never links task code:

```rust
#[task(single)]
pub fn analyze(doc: String, min_len: i32) -> Result<Stats>;
```

## In-process tasks (SimpleEngine)
For local testing and embedding, write the task with `#[task]` and register it:

```rust
#[task(single)]
pub fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}

register_task! { method = add, name = "add", author = "Namu", version = "0.1.0" }
```

## Task kinds
- `single`: one input, one output
- `batch`: vectorized inputs and outputs (in-process only today)
- `stream`: one input, many outputs; each output continues in its own branch
