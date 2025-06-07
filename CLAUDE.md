# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

NAMU is a high-performance Rust pipeline engine built as a multi-crate workspace. The project follows a distributed architecture with distinct components for different pipeline responsibilities.

## Development Commands

### Building and Testing

- `cargo build` - Build the entire workspace
- `cargo test` - Run tests across all crates
- `cargo check` - Type check all crates
- `cargo build --bin namu` - Build only the CLI binary

### CLI Usage

- `cargo run --bin namu compile` - Compile tasks from ./tasks to ./outputs/tasks for multiple targets
- `cargo run --bin namu check` - Type check all task definitions
- `cargo run --bin namu compile --tasks-dir ./custom/path` - Compile tasks from custom directory
- `cargo run --bin namu compile --use-subdirs` - Use target subdirectories instead of filename suffixes

### Running Examples

- `cargo run --example simple --manifest-path crates/graph/Cargo.toml` - Run graph simple example
- `cargo run --example workflow --manifest-path crates/graph/Cargo.toml` - Run workflow example
- `cargo run --example conditional --manifest-path crates/graph/Cargo.toml` - Run conditional example

## Architecture

The codebase is organized as a Cargo workspace with six main crates:

- **cli**: Command-line interface (`namu` binary) that compiles tasks to cross-platform binaries and performs type checking. Supports building for multiple targets (aarch64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu).
- **master**: Master node responsible for orchestrating and coordinating pipeline execution.
- **worker**: Worker nodes that execute pipeline tasks.
- **graph**: Core graph processing library with `TraceNode<T>` for building computational graphs. Uses the `#[trace]` macro to convert functions into graph nodes. Supports conditional execution with `graph_if`.
- **task**: Rust traits for defining pipeline tasks.
- **macros**: Procedural macros including `#[trace]` for automatic graph node generation and `#[workflow]` for pipeline definitions.

### Task System

Tasks are independent Rust binaries in the `tasks/` directory, each with their own `Cargo.toml`. They are excluded from the main workspace to allow standalone compilation. The CLI compiles these tasks into cross-platform binaries for distribution.

### Graph System

The graph system uses a functional approach where functions decorated with `#[trace]` automatically become nodes in a computational graph. The `TraceNode<T>` type maintains type safety while allowing graph composition and execution.
