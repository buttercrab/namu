# Contributing

Thanks for helping improve NAMU.

## Before you start
- Use Rust stable for development.
- Run the end-to-end pipeline once: `./scripts/e2e.sh`.

## Code style
- Format: `cargo +nightly fmt --all --check`
- Lint: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

## Tests
- Unit + integration: `cargo test --workspace --all-targets --all-features`
- CI parity: `./scripts/verify.sh`

## Commit messages
- Short, imperative, and lowercase (example: `add worker cache`).

## Pull requests
Include:
- Summary of changes
- Tests run (copy the command output)
- Behavior changes or migrations

## Repo conventions
- Docs live in `docs/`.
- Task/workflow fixtures for e2e live in `tests/e2e/`.
- See `AGENTS.md` for agent-specific instructions.
