# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

NAMU is a high-performance Rust pipeline engine built as a multi-crate workspace. The project follows a distributed architecture with distinct components for different pipeline responsibilities.

## Architecture

The codebase is organized as a Cargo workspace with six main crates:

- **cli**: Command-line interface for interacting with the pipeline system. Which mainly compile tasks to executable binaries and check types for pipeline definitions.
- **master**: Master node responsible for orchestrating and coordinating pipeline execution.
- **worker**: Worker nodes that execute pipeline tasks.
- **graph**: Graph processing and pipeline topology management.
- **task**: Rust traits for tasks.
- **macros**: macros and code generation utilities.
