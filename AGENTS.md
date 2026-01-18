# Repository Guidelines

## Project Structure & Module Organization
- Root workspace holds shared config and top-level crate (`Cargo.toml`, `src/`).
- Core Rust crates live in `crates/`: `core`, `flow`, `macros`, `engine`, `cli`, `master`, `worker`.
- Examples are under `examples/` (e.g., `examples/simple`).
- Tests are in `tests/`, including compile-fail cases in `tests/compile-fail`.
- CI workflows live in `.github/workflows/`.

## Build, Test, and Development Commands
- `cargo build --workspace` — build all crates in the workspace.
- `cargo check --workspace --all-targets --all-features` — fast type-check across targets and features.
- `cargo test --workspace --all-targets --all-features` — run the full test suite, including trybuild tests.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — lint with warnings as errors.
- `cargo +nightly fmt --all --check` — enforce formatting (nightly rustfmt).
- `cargo doc --workspace --all-features --no-deps` — ensure docs build without external deps.

## Coding Style & Naming Conventions
- Rust formatting follows `rustfmt` (nightly in CI). Use standard Rust 4-space indentation.
- Naming: `snake_case` for functions/modules, `CamelCase` for types/traits, `SCREAMING_SNAKE_CASE` for consts.
- Prefer explicit types where they clarify macro-generated code paths.

## Testing Guidelines
- Tests use `cargo test` with trybuild for macro compile-fail checks (`tests/compile-fail/*.rs`).
- Add unit tests for macro expansions, IR serialization, and engine execution paths when changing behavior.
- Name tests descriptively (e.g., `engine_executes_fibonacci_workflow`).

## Commit & Pull Request Guidelines
- History shows short, lowercase, imperative-style messages (e.g., `fix ci`, `add tests`).
- Keep commits concise; include a scope if helpful (e.g., `fix macros: task defaults`).
- PRs should describe the change, testing performed, and any behavior impacts.

## Security & Configuration Tips
- `cargo deny` and `cargo audit` are part of CI. Keep license fields updated in all `Cargo.toml` files.
- Prefer workspace dependency versions to avoid wildcard path dependencies.
