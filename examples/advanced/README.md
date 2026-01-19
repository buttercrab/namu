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

## Export workflow JSON (for orchestrator)

```bash
cargo run -p namu-advanced-workflows --bin export
```

By default this writes to `examples/advanced/dist/workflows`.

## Build & publish to orchestrator

```bash
cargo run -p namu-cli -- build \
  --tasks-dir examples/advanced/tasks \
  --workflows-dir examples/advanced/dist/workflows \
  --out-dir examples/advanced/dist

NAMU_ORCH_URL=http://localhost:8080 \
  cargo run -p namu-cli -- publish --out-dir examples/advanced/dist
```

## Value refs & large payloads (optional)

To exercise value refs, set a low inline limit in your worker/orchestrator environment:

```bash
export NAMU_INLINE_INPUT_LIMIT_BYTES=32
```

Then run a workflow that passes larger values; the orchestrator will store them in the object store
and workers will fetch by reference.
