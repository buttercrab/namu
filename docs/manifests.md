# Task & Workflow Manifests

NAMU expects tasks and workflows to be described via JSON files. The CLI reads these when building artifacts.

## Task manifest (`manifest.json`)
Located in each task directory. Example:
```json
{
  "task_id": "add",
  "version": "0.1.0",
  "task_kind": "single",
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

## Workflow upload payload
`namu build` copies workflow JSON into `dist/workflows/` and `namu publish` uploads it.

Required fields:
- `id`: workflow identifier
- `version`: workflow version
- `ir`: JSON IR emitted by `#[workflow]`
- `task_versions`: map of task_id -> version
