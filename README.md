# NAMU – Composable, AI-first Pipeline Engine for Rust

_NAMU turns ordinary Rust functions into distributed, type-safe data pipelines._  
Developers annotate compute units with `#[task]`, orchestrate them in `#[workflow]` functions, and let the compile-time tooling generate an immutable control-flow graph that runs efficiently on the **namu-engine** at runtime.

---

## High-Level Architecture

```
Rust Source        ──►  namu-macros  ──►  JSON IR  ──►  namu-engine  ──►  Workers
(tasks + workflow)         │                               ▲
                           │                               │
                   namu-flow (graph builder)         namu-master (registry)
```

- **Compile-time**: Procedural macros rewrite user code into a Static-Single-Assignment graph and serialise it as JSON.
- **Runtime**: The engine interprets the graph, streams immutable _contexts_ through tasks, and scales out across workers under the master's supervision.

---

## Workspace Layout (Crate Map)

| Crate / Service | Role                                                              |
| --------------- | ----------------------------------------------------------------- |
| **namu-core**   | Public traits (`Task`, `TaskContext`) and serialisable IR structs |
| **namu-macros** | Implements `#[task]` and `#[workflow]`                            |
| **namu-flow**   | Compile-time graph builder used by macros                         |
| **namu-engine** | Runtime interpreter; manages contexts, dispatches tasks           |
| **namu-cli**    | Developer tooling (`compile`, `login`, `publish`)                 |
| **namu-master** | HTTP + WebSocket registry / control-plane                         |
| **namu-worker** | Lightweight node that executes tasks                              |

---

## Developer Quick-Start (Illustrative)

```rust
// tasks/add/src/lib.rs
use namu::prelude::*;

#[task]
fn add(a: i32, b: i32) -> Result<i32> { Ok(a + b) }
```

```rust
// workflow/src/lib.rs
use namu::prelude::*;

#[workflow]
fn compute() -> i32 {
    let x = add(1, 2);
    add(x, 40)
}
```

```bash
namu compile   # type-checks every task crate with the `no-run` feature
namu publish   # uploads sources to the registry (namu-master)
```

Workers connected to the master will now execute `compute` in parallel.

---

## Design Documents

- **[design.mdc](design.mdc)** – strategic, high-level overview.
- **[namu.mdc](namu.mdc)** – in-depth implementation guide.

For questions or contributions, please open an issue or join the discussion board.
