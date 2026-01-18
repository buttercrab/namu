#!/usr/bin/env bash
set -euo pipefail

if ! command -v cargo-deny >/dev/null; then
  echo "cargo-deny is required: cargo install cargo-deny --locked" >&2
  exit 1
fi

if ! command -v cargo-audit >/dev/null; then
  echo "cargo-audit is required: cargo install cargo-audit --locked" >&2
  exit 1
fi

if ! command -v cargo-expand >/dev/null; then
  echo "cargo-expand is required: cargo install cargo-expand --locked" >&2
  exit 1
fi

cargo +nightly fmt --all --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo doc --workspace --all-features --no-deps

cargo deny check --deny warnings
cargo audit --deny warnings
