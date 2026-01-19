# Intermediate Representation (IR)

The IR is a JSON description of a workflow graph produced by `#[workflow]` and consumed by the engine. It is deterministic, compact, and language-agnostic.

## What it contains
- **Operations**: `Literal`, `Call`, `Phi`, `Extract`
- **Outputs**: SSA value ids produced by each operation
- **Control flow**: `Jump`, `Branch`, `Return`

## Example (simplified)
```json
{
  "kind": { "Call": { "task_id": "add", "inputs": [0, 1] } },
  "outputs": [2],
  "next": { "Jump": { "next": 5 } }
}
```

## Key properties
- One producer per value id (SSA).
- No runtime reflection; the engine interprets the JSON directly.
- Serializable with `serde` and stable across releases.

## Where it lives
- Types are defined in `crates/libs/core`.
- The `namu-flow` builder emits the serialized graph.
- The engine consumes it via `namu-engine`.
