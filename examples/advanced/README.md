# Advanced Example

This example is a compact "tour" of Namu features using three themed pipelines:

- **ETL pipeline**: stream → normalize → score → branch + loop
- **ML pipeline**: stream → batch embed → score → branch
- **Media pipeline**: stream → fail-on-leaf → score

All tasks are intentionally simple (integer math) so the workflow structure is easy to follow.

## Structure

- `tasks/` — individual task crates (single, batch, stream).
- `workflows/` — workflow definitions plus small helpers to run/export them.
- `dist/workflows/` — generated `.workflow.json` files (created by the export helper).

## Run in-process (SimpleEngine)

```bash
cargo run -p namu-advanced-workflows --bin run -- etl
cargo run -p namu-advanced-workflows --bin run -- ml
cargo run -p namu-advanced-workflows --bin run -- media
```

## Distributed execution

These tasks use the in-process `#[task]` / `register_task!` API and run on the `SimpleEngine`.
For tasks that run on workers (WASM components and native libraries), see `examples/distributed`.
