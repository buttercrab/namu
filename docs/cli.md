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
- `namu run <workflow_id> <version>`
  - Creates a run for a workflow version.
- `namu status <run_id>`
  - Returns run status and progress counts.
- `namu logs <run_id> --limit 100`
  - Fetches recent run events.
- `namu workers`
  - Lists registered workers.
- `namu login`
  - Stores orchestrator URL after a health check.

## Example: build and publish the e2e fixtures
```bash
cargo run -p namu-cli -- build --tasks-dir tests/e2e/tasks --workflows-dir tests/e2e/workflows --out-dir tests/e2e/dist
cargo run -p namu-cli -- publish --out-dir tests/e2e/dist
```

## Example: advanced workflows
The advanced example exports workflow IR from Rust code.
```bash
cargo run -p namu-advanced-workflows --bin export -- --out examples/advanced/dist/workflows
cargo run -p namu-cli -- build --tasks-dir examples/advanced/tasks --workflows-dir examples/advanced/dist/workflows --out-dir examples/advanced/dist
cargo run -p namu-cli -- publish --out-dir examples/advanced/dist
```
