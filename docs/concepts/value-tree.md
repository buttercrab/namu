# Value Tree

Each workflow run produces immutable values. NAMU stores them in a tree of contexts so that
parallel branches share what they have in common, without copying it and without locks. See
`design.md` §6 for the full model.

## Core idea
- A **context** is one execution path. Each step that produces values adds a new node: a parent
  pointer and the values the step produced.
- **Nodes never change**, including their parent. Lookups walk up the ancestors.
- A **fan-out** creates one child per item; siblings share everything above them.
- A **collect** continues in a new child of the context that fanned out: values from before the
  fan-out are visible again, per-item values are not.

- **Constants** depend on nothing, so they are placed once, in the root context, where every branch
  and scope sees them.

## In the engine
- A node lives on the worker that created it; its id names that worker. Other workers fetch it
  from there once and cache it, which is always safe because it never changes. A handoff carries
  the branch's own context node, so the receiver usually needs no fetch at all.
- Which scopes a branch is in is part of the branch, not of the tree. Each fan-out scope is owned
  by the worker that opened it, and ends by weight throwing: branches return their share of the
  scope's weight when they end, and the scope is over when all of it is back.
- Which collect closes which fan-out is found from the IR before the run, so a scope with no
  items, or whose items were all dropped, collects `[]`.

## In the proof-of-concept worker
- Nodes live in Redis. A worker writes the nodes it creates behind the critical path, and flushes
  before handing work to another worker.
- Nodes produced by another worker are fetched once and cached. They are immutable, so the cache
  never needs invalidating.
- Work items only reference a context; values are always read through the tree.
- Fan-in counts open branches per scope (`live`). The branch that brings it to zero continues the
  workflow with the collected list.
