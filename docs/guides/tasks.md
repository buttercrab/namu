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

Inputs and outputs are JSON. With no arguments the input is `null`, with one it is the value
itself, and with several it is an array. One task per crate. See `examples/distributed/tasks`.

The workflow crate declares the task's signature without a body, so it never links task code:

```rust
#[task]
pub fn analyze(doc: String, min_len: i32) -> Result<Stats>;
```

## In-process tasks (`FnTasks`)
In the same process as the engine, a task is a Rust function registered by name. Its input is the
packed arguments: the argument itself for one, a tuple for several, `()` for none.

```rust
let tasks = FnTasks::new()
    .with("add", |(a, b): (i32, i32)| Ok::<_, String>(a + b));
```

See `examples/basic`.

## Task kinds
- `single`: one input, one output
- `batch`: written for a list of inputs, called per item; the engine groups items (design §6.6,
  not implemented yet)
- `stream`: one input, many outputs; each output continues in its own branch
