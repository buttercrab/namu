# Distributed example

Tasks and workflows for the distribution proof of concept (`docs/poc.md`).

- `tasks/`: one crate per task, written with `namu_guest::task!`. `manifest.json` sets the runtime:
  everything is WASM except `summarize` and `inc_native`, which are native libraries.
- `workflows/`: the workflows, which reference tasks through bodiless `#[task]` signatures. The
  `export` binary writes their IR as upload payloads.

```bash
./scripts/poc-local.sh                      # local cluster, doc_stats
WORKFLOW=nested_stats ./scripts/poc-local.sh
./scripts/poc-cross.sh                      # with Linux workers in Docker
./scripts/e2e.sh                            # asserted end-to-end check
```
