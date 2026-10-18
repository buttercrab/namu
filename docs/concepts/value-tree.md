# Value Tree

Each workflow run produces immutable values. NAMU stores them in a tree of contexts so that
parallel branches share what they have in common, without copying it and without locks. See
`design.md` §6 for the full model.

## Core idea
- A **context** is one execution path. Each step adds a new node: a parent pointer, the values the
  step produced, and the enclosing fan-out scopes.
- **Nodes never change**, including their parent. Lookups walk up the ancestors.
- A **fan-out** creates one child per item; siblings share everything above them.
- A **collect** continues in a new child of the context that fanned out: values from before the
  fan-out are visible again, per-item values are not.

## In the distributed runtime
- Nodes live in Redis. A worker writes the nodes it creates behind the critical path, and flushes
  before handing work to another worker.
- Nodes produced by another worker are fetched once and cached. They are immutable, so the cache
  never needs invalidating.
- Work items only reference a context; values are always read through the tree.
- Fan-in counts open branches per scope (`live`). The branch that brings it to zero continues the
  workflow with the collected list.
