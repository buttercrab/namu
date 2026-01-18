# Contributing

Thanks for helping improve NAMU.

## Development setup
1. Install Rust (stable).
2. Optional: Docker for local Postgres + Redis.
3. Run `./scripts/e2e.sh` to validate the full pipeline locally.

## Code style
- Run `cargo +nightly fmt --all --check` before submitting.
- Treat clippy warnings as errors: `cargo clippy --workspace --all-targets --all-features -- -D warnings`.

## Tests
- Run the full suite: `cargo test --workspace --all-targets --all-features`.
- CI parity: `./scripts/verify.sh`.

## Commit messages
- Keep messages short and imperative (e.g., `fix ci`, `add worker metrics`).

## Pull requests
Include:
- A short summary of changes
- Tests run (copy/paste the commands)
- Notes on behavior changes or migrations (if any)

## Automation notes
- `AGENTS.md` contains repository-specific instructions for coding agents.
