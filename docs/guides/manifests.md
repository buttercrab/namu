# Manifests

NAMU uses JSON manifests for tasks and workflow uploads.

## Task manifest (`manifest.json`)
Each task directory must include a `manifest.json` that matches `namu_proto::TaskManifest`.

Example:
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

Rules enforced by the orchestrator:
- `trust = untrusted` requires `runtime = wasm`.
- `runtime = wasm` requires `trust = untrusted`.
- `requires_gpu = true` requires `runtime = native` and `trust != untrusted`.

`checksum` is filled in by `namu build` when packaging artifacts.

## Workflow upload payload
`namu build` copies workflow IR JSON files into `dist/workflows/`. `namu publish` uploads them as:

```json
{
  "id": "etl_pipeline",
  "version": "0.1.0",
  "ir": { "operations": [] },
  "task_versions": { "add": "0.1.0" }
}
```

Workflow files must be named `*.workflow.json`.
