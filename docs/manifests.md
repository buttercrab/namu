# Task & Workflow Manifests

NAMU expects tasks and workflows to be described via JSON files. The CLI reads these when building artifacts.

## Task manifest (`manifest.json`)
Located in each task directory. Example:
```json
{
  "task_id": "add",
  "version": "0.1.0",
  "task_kind": "single",
  "trust": "trusted",
  "runtime": "native",
  "requires_gpu": false,
  "resource_class": "cpu.small",
  "capabilities": ["cpu"],
  "input_arity": 2,
  "output_arity": 1,
  "input_schema": { "type": "array", "items": [ {"type": "integer"}, {"type": "integer"} ] },
  "output_schema": { "type": "integer" },
  "checksum": "",
  "abi_version": "1",
  "build_toolchain": "rust-1.75.0",
  "created_at": "2026-01-18T00:00:00Z"
}
```
Notes:
- `checksum` is filled by `namu build` after packaging the artifact.
- `task_kind` supports `single`, `batch`, `stream`.
- `input_schema` / `output_schema` are JSON Schema fragments (free-form).
- `trust` controls routing: `trusted`, `restricted`, or `untrusted` (untrusted routes to WASM pool).
- `runtime` declares artifact type: `native` or `wasm`.
- `requires_gpu` routes tasks to the GPU pool.
- Tasks fail to queue if no worker is registered for the selected pool + `resource_class`.
- `runtime = "wasm"` expects a `.wasm` artifact built for `wasm32-wasip1` (fallback: `wasm32-wasi`).
- `trust = "untrusted"` must use `runtime = "wasm"`; GPU tasks must be native and not untrusted.

## Workflow upload payload
`namu build` copies workflow JSON into `dist/workflows/` and `namu publish` uploads it.

Required fields:
- `id`: workflow identifier
- `version`: workflow version
- `ir`: JSON IR emitted by `#[workflow]`
- `task_versions`: map of task_id -> version
