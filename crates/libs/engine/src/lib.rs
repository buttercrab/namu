//! The NAMU engine.
//!
//! Workers interpret workflow IR themselves. A branch runs on one worker for as long as that
//! worker serves the tasks it calls, and moves to another worker only when it must. Values live
//! in an immutable tree whose nodes are held by the workers that created them; fan-out scopes
//! are owned by the workers that opened them and end once all their branches have. See
//! `docs/design.md`.

mod cluster;
pub mod ids;
pub mod kernel;
pub mod log;
pub mod message;
mod persistent;
pub mod routes;
pub mod run;
pub mod scope;
pub mod tasks;
pub mod transport;
pub mod tree;
mod worker;

pub use cluster::LocalCluster;
pub use run::{RunInfo, RunOutcome, RunSpec, WorkerStats};
pub use tasks::{CallKey, FnTasks, TaskRef, Tasks, call_key};
pub use worker::Worker;
