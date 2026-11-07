# CLI

Run `namu --help` for the full command list. When working from source, use `cargo run -p namu-cli -- <args>`.

## Core commands
- `namu build --tasks-dir <dir> --workflows-dir <dir> --out-dir <dir>`
  - Builds task artifacts and copies workflow IR files into the output directory.
- `namu build --config ./namu.toml`
  - Builds using `namu.toml` and auto-exports workflow IR.
- `namu sync --config ./namu.toml`
  - Syncs task dependencies into `Cargo.toml` and registry entries into `.cargo/config.toml`.
- `namu publish --out-dir <dir>`
  - Uploads artifacts and workflow IR to the orchestrator.
- `namu run <workflow_id> <version> [--input NAME=VALUE]... [--wait]`
  - Creates a run; with `--wait`, waits and prints the status and results. Exits non-zero if the
    run did not succeed.
  - `--input` gives a workflow parameter, once per parameter. The value is JSON (`limit=300`,
    `tags=["a","b"]`, `id="42"` for a string of digits), or else a string (`collection=wikipedia`).
    Inputs that do not fit the workflow are refused before the run starts.
- `namu status <run_id>`
  - Returns run status, how many results and failures it has so far, and task calls per worker.
- `namu results <run_id> [--follow] [--failures]`
  - Prints the run's results (or failures), one JSON value per line, in the order they arrived.
    With `--follow`, keeps printing them as they arrive until the run ends: an unbounded run's
    results are only ever read this way. The API underneath is
    `GET /runs/{id}/results?after=N&limit=M` (and `/failures`), pages with a `next` cursor.
- `namu workers`
  - Lists registered workers.
- `namu login`
  - Stores orchestrator URL after a health check.

## Example: the distributed example
```bash
cargo run -p namu-distributed-workflows --bin export -- examples/distributed/dist/workflows
cargo run -p namu-cli -- build --tasks-dir examples/distributed/tasks \
  --workflows-dir examples/distributed/dist/workflows --out-dir examples/distributed/dist
cargo run -p namu-cli -- publish --out-dir examples/distributed/dist
cargo run -p namu-cli -- run doc_stats 0.1.0 --wait
```
