# Engine

The engine interprets workflow IR, manages contexts, and dispatches task calls.

## The kernel
`EngineKernel` (in `namu-engine`) is shared by every engine. It walks operations, applies literals
and phis as one step (phis read all their sources before writing, so swaps work), and stops at the
next action:
- `Dispatch`: a task call,
- `FanOut`: split into one branch per list item,
- `Collect`: this branch reached a fan-in,
- `Return`.

The kernel is generic over a `ValueStore`, so each engine brings its own storage.

## Engines
- **namu-worker** (distributed): each worker drives contexts with the kernel, stores values in the
  immutable value tree (Redis), runs WASM and native tasks, and hands work to other workers. See
  `poc.md`.
- **SimpleEngine** (in-process): thread-per-task with channels, for tests and embedding. It does not
  support fan-out and collect yet.
