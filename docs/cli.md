# CLI

Run `namu --help` for the full command list. The CLI is oriented around three core phases: build, publish, run.

## Commands
- `namu build --tasks-dir ./tasks --workflows-dir ./workflows --out-dir ./dist`
  - Builds task artifacts and copies workflow IR files into the output directory.
- `namu publish --out-dir ./dist`
  - Uploads artifacts + workflow IR to the orchestrator.
- `namu run <workflow_id> <version>`
  - Creates a new workflow run.
- `namu status <run_id>`
  - Returns run status and progress counts.
- `namu logs <run_id> --limit 100`
  - Fetches recent run events.
- `namu workers`
  - Lists registered workers.
- `namu login`
  - Stores orchestrator URL after a health check.
- `namu version`
  - Prints CLI version.

## Environment variables
- `NAMU_ORCH_URL`: orchestrator base URL (used by CLI and worker).

## Example session
```bash
namu build   --tasks-dir ./tasks --workflows-dir ./workflows --out-dir ./dist
namu publish --out-dir ./dist
namu run add_workflow 0.1.0
namu status <run_id>
namu logs <run_id> --limit 200
```
