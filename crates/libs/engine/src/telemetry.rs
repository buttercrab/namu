//! What a worker tells about itself, for whoever listens: metrics through the `metrics` facade,
//! spans through `tracing`. Nothing is recorded until the process installs a recorder or a
//! subscriber (`namu-telemetry` does, in the binaries).
//!
//! Metrics carry no run ids: there would be one series per run. A task's id is fine: there are
//! as many as a cluster serves.
//!
//! With the `otel` feature, a run is one trace across every worker, without anything carried in
//! messages: its trace id is the run's id, and every task span is a child of the run's root span,
//! whose id comes from the run's id too ([`run_trace`]). The control plane exports that root.

use std::time::Duration;

#[cfg(feature = "otel")]
use crate::ids::RunId;
use crate::tasks::{CallKey, TaskRef};

/// Describes the engine's metrics to the recorder, for its help texts.
pub fn describe() {
    use metrics::{Unit, describe_counter, describe_gauge, describe_histogram};
    describe_counter!(
        "namu_task_calls_total",
        "Task calls, by task and outcome (ok, failed, error)"
    );
    describe_histogram!(
        "namu_task_call_seconds",
        Unit::Seconds,
        "How long task calls took, by task"
    );
    describe_counter!(
        "namu_task_retries_total",
        "Failed task calls tried again, by task (see a task's retry policy)"
    );
    describe_counter!(
        "namu_handoffs_sent_total",
        "Branches handed to another worker"
    );
    describe_counter!(
        "namu_handoffs_received_total",
        "Branches handed here by another worker"
    );
    describe_counter!(
        "namu_branches_ended_total",
        "Branches that ended in a scope this worker owns"
    );
    describe_counter!(
        "namu_node_fetches_total",
        "Value-tree nodes fetched from other workers"
    );
    describe_counter!(
        "namu_reports_total",
        "Reports recorded by runs' owners here, by kind"
    );
    describe_gauge!("namu_runs", "Runs this worker takes part in");
    describe_gauge!(
        "namu_scopes_open",
        "Fan-out scopes this worker owns that are open"
    );
    describe_gauge!(
        "namu_branches_driving",
        "Branches being driven here: the load it reports"
    );
    describe_gauge!("namu_task_slots_busy", "Task slots in use");
    describe_gauge!(
        "namu_unacknowledged_messages",
        "Messages kept until their receivers acknowledge them"
    );
    describe_gauge!(
        "namu_releases_pending",
        "Log records waiting for acknowledgements to go"
    );
}

/// A task call: one span, and its outcome and duration in metrics.
pub(crate) fn task_span(key: &CallKey, task: &TaskRef) -> tracing::Span {
    let span = tracing::info_span!(
        "task",
        task = %task.id,
        version = %task.version,
        run = %key.run,
        call = %key,
    );
    #[cfg(feature = "otel")]
    {
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        let _ = span.set_parent(run_context(key.run));
    }
    span
}

/// How a call ended, for `namu_task_calls_total`.
pub(crate) enum Outcome {
    /// The task returned a value.
    Ok,
    /// The task returned an error: the branch fails, or matches on it.
    Failed,
    /// The call did not happen as asked: the task could not be run.
    Error,
}

pub(crate) fn task_called(task: &TaskRef, outcome: Outcome, took: Duration) {
    let outcome = match outcome {
        Outcome::Ok => "ok",
        Outcome::Failed => "failed",
        Outcome::Error => "error",
    };
    metrics::counter!("namu_task_calls_total", "task" => task.id.clone(), "outcome" => outcome)
        .increment(1);
    metrics::histogram!("namu_task_call_seconds", "task" => task.id.clone())
        .record(took.as_secs_f64());
}

pub(crate) fn task_retried(task: &TaskRef) {
    metrics::counter!("namu_task_retries_total", "task" => task.id.clone()).increment(1);
}

pub(crate) fn handoff_sent() {
    metrics::counter!("namu_handoffs_sent_total").increment(1);
}

pub(crate) fn handoff_received() {
    metrics::counter!("namu_handoffs_received_total").increment(1);
}

pub(crate) fn branch_ended() {
    metrics::counter!("namu_branches_ended_total").increment(1);
}

pub(crate) fn node_fetched() {
    metrics::counter!("namu_node_fetches_total").increment(1);
}

pub(crate) fn report_recorded(failure: bool) {
    let kind = if failure { "failure" } else { "result" };
    metrics::counter!("namu_reports_total", "kind" => kind).increment(1);
}

/// A worker's state, sampled: what it holds right now.
pub(crate) struct Sample {
    pub runs: usize,
    pub scopes_open: usize,
    pub branches_driving: u32,
    pub task_slots_busy: usize,
    pub unacknowledged: usize,
    pub releases_pending: usize,
}

pub(crate) fn sampled(sample: Sample) {
    metrics::gauge!("namu_runs").set(sample.runs as f64);
    metrics::gauge!("namu_scopes_open").set(sample.scopes_open as f64);
    metrics::gauge!("namu_branches_driving").set(f64::from(sample.branches_driving));
    metrics::gauge!("namu_task_slots_busy").set(sample.task_slots_busy as f64);
    metrics::gauge!("namu_unacknowledged_messages").set(sample.unacknowledged as f64);
    metrics::gauge!("namu_releases_pending").set(sample.releases_pending as f64);
}

/// How often a worker samples its state.
pub(crate) const SAMPLE_EVERY: Duration = Duration::from_secs(1);

/// A run's trace: its id, and its root span's. Every worker, and the control plane, arrive at
/// the same two without asking anyone.
#[cfg(feature = "otel")]
pub fn run_trace(run: RunId) -> (opentelemetry::trace::TraceId, opentelemetry::trace::SpanId) {
    let bytes = run.into_bytes();
    let mut root = [0u8; 8];
    root.copy_from_slice(&bytes[8..]);
    // A span id may not be all zeros.
    if root == [0; 8] {
        root[7] = 1;
    }
    (
        opentelemetry::trace::TraceId::from_bytes(bytes),
        opentelemetry::trace::SpanId::from_bytes(root),
    )
}

/// The run's root span, as a remote parent: a run is traced, or not, as a whole (`sampled`).
#[cfg(feature = "otel")]
pub fn run_context(run: RunId) -> opentelemetry::Context {
    use opentelemetry::trace::{SpanContext, TraceContextExt, TraceFlags, TraceState};
    let (trace, root) = run_trace(run);
    let flags = match sampled_run(run) {
        true => TraceFlags::SAMPLED,
        false => TraceFlags::default(),
    };
    let parent = SpanContext::new(trace, root, flags, true, TraceState::default());
    opentelemetry::Context::new().with_remote_span_context(parent)
}

/// The share of runs traced, in millionths: every worker decides alike from the run's id.
#[cfg(feature = "otel")]
static TRACE_RATIO: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1_000_000);

/// Traces `ratio` (0 to 1) of runs, chosen by their ids.
#[cfg(feature = "otel")]
pub fn trace_runs(ratio: f64) {
    let millionths = (ratio.clamp(0.0, 1.0) * 1e6) as u32;
    TRACE_RATIO.store(millionths, std::sync::atomic::Ordering::Relaxed);
}

/// Whether `run` is traced.
#[cfg(feature = "otel")]
pub fn sampled_run(run: RunId) -> bool {
    let ratio = TRACE_RATIO.load(std::sync::atomic::Ordering::Relaxed);
    let draw = u32::from_be_bytes(run.as_bytes()[12..16].try_into().expect("4 bytes")) % 1_000_000;
    draw < ratio
}
