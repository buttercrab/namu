# Value Tree

Each workflow run produces immutable values. NAMU stores them in a tree of contexts so that
parallel branches share what they have in common, without copying it and without locks. See
`design.md` §6 for the full model.

## Core idea
- A **context** is one execution path. Each step that produces values adds a new node: a parent
  pointer and the values the step produced.
- **Nodes never change**, including their parent. Lookups walk up the ancestors.
- **A new node attaches under the deepest node that still holds a live value**, not blindly under
  the current one. Which values are live where is found from the IR before the run; a node's own
  values shadow older copies of the same value (a loop's phi defines the same value every
  iteration). So a chain is as deep as the values in use, however long the run, and the nodes left
  behind hold only dead values (design §6.2).
- A **fan-out** creates one child per item; siblings share everything above them.
- A **collect** continues in a new child of the context that fanned out: values from before the
  fan-out are visible again, per-item values are not.

- **Constants** depend on nothing, so they are placed once, in the root context, where every branch
  and scope sees them.

## In the engine
- A node owns its parent. The contexts in use (branches being driven, scopes waiting to
  continue, streams, branches waiting for a batch) keep their chains alive, and a node nothing
  reaches is freed at once. A freed node asked for again (by another worker, say) is read from the
  log, which holds every node a live branch can reach.
- A node's id names the worker that created it, which serves it to other workers while it is
  alive. A worker that fetches a node shares it among its own branches, which is always safe
  because it never changes. A handoff carries the branch's own context and the nodes it hangs
  from, so the receiver usually needs no fetch at all.
- Which scopes a branch is in is part of the branch, not of the tree. Each fan-out scope is owned
  by the worker that opened it, which created all of its branches: it counts them as they end,
  and the scope is over when all have.
- Which collect closes which fan-out is found from the IR before the run, so a scope with no
  items, or whose items were all dropped, collects `[]`.
