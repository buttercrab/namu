# Intermediate Representation (IR)

The IR is a JSON description of a workflow, produced by `#[workflow]` and run by the engine. It is
deterministic, compact, and language-agnostic.

## What it contains
A workflow is a list of **operations**. Each operation does, in order:

1. **Literals**: constants, as JSON text.
2. **Phis**: values chosen by the operation control came from, applied together (all sources are
   read before any is written).
3. **Built-ins**, in order: expressions the engine evaluates itself, such as
   `{ "op": "add", "ty": "i64", "inputs": [3, 4], "output": 5 }`. `ty` is the operands' type.
4. At most one **action**: `Call` (a task), `FanOut` (one branch per item of a list), or
   `Collect` (gather one value from every branch of the innermost fan-out).
5. **Next**: `Jump`, `Branch` on a bool, or `Return`.

A `Call` may have an error edge, `catch: { "error": e, "next": op }`, as LLVM's `invoke` does: if
the task fails, its outputs are not defined; its error message is bound to `e` and control goes to
`op` instead of `next`. Without one, a failure fails the branch. `match attempt(..)` emits it.

Values are SSA ids: each is produced once.

## Types
Every value has a type: `types[v]` is value `v`'s. Standard types are structural (`"bool"`,
`{ "int": { "bits": 32, "signed": true } }`, `"string"`, `{ "list": ... }`, ...); structs and enums
are `{ "named": "crate::module::Name" }`, with their shapes in `defs`:

```json
"defs": {
  "my_crate::Chunk": { "struct": [["doc_id", { "int": { "bits": 64, "signed": false } }],
                                   ["text", "string"]] }
}
```

## Example
```json
{
  "literals": [ { "output": 2, "value": "3" } ],
  "phis": [ { "output": 1, "from": [[0, 0], [2, 5]] } ],
  "action": { "Call": { "task_id": "less_than", "inputs": [1, 2], "outputs": [3] } },
  "next": { "Branch": { "var": 3, "true_next": 2, "false_next": 3 } }
}
```

## Key properties
- One producer per value id (SSA).
- No runtime reflection; the engine interprets the IR directly.
- Which collect closes which fan-out, and which values are live where, are computed from the IR
  before a run (`namu_core::scopes`, `namu_core::liveness`).

## Where it lives
- Types are defined in `crates/libs/core` (`ir.rs`).
- The `namu-flow` builder emits it.
- The engine's kernel interprets it.
