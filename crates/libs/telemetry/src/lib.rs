//! Logs, metrics and traces for a NAMU process, as its environment asks:
//!
//! - **Logs** go to stderr: text, or JSON lines with `NAMU_LOG_FORMAT=json` (for CloudWatch Logs
//!   and the like). `RUST_LOG` filters them (default `info`).
//! - **Metrics** are served for Prometheus at `http://<NAMU_METRICS_ADDR>/metrics`, if that is set.
//!   The engine's are listed in `docs/operations/observability.md`.
//! - **Traces** go to an OpenTelemetry collector over OTLP/HTTP, if
//!   `OTEL_EXPORTER_OTLP_ENDPOINT` is set (e.g. `http://localhost:4318`, an ADOT sidecar). A run is
//!   one trace across every worker; `NAMU_TRACE_RATIO` (0 to 1, default 1) is the share of runs
//!   traced.

use std::cell::Cell;
use std::net::SocketAddr;
use std::time::SystemTime;

use anyhow::Context;
use opentelemetry::KeyValue;
use opentelemetry::trace::{Span as _, SpanId, TraceId, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{IdGenerator, RandomIdGenerator, Sampler, SdkTracerProvider};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// What was set up; traces still buffered are sent when it is dropped.
pub struct Telemetry {
    traces: Option<SdkTracerProvider>,
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        if let Some(traces) = self.traces.take()
            && let Err(err) = traces.shutdown()
        {
            eprintln!("sending the last traces failed: {err}");
        }
    }
}

/// Sets up telemetry for the process `service` (`namu-master`, `namu-worker`). Call it once,
/// inside the Tokio runtime, before anything logs.
pub fn init(service: &'static str) -> anyhow::Result<Telemetry> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());

    let traces = match var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        Some(_) => {
            if let Some(ratio) = var("NAMU_TRACE_RATIO") {
                let ratio: f64 = ratio.parse().context("NAMU_TRACE_RATIO")?;
                namu_engine::telemetry::trace_runs(ratio);
            }
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .build()
                .context("the OTLP exporter")?;
            let provider = tracer_provider(service, exporter);
            opentelemetry::global::set_tracer_provider(provider.clone());
            Some(provider)
        }
        None => None,
    };

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let logs = match var("NAMU_LOG_FORMAT").as_deref() {
        Some("json") => tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(true)
            .boxed(),
        Some("text") | None => tracing_subscriber::fmt::layer().boxed(),
        Some(other) => anyhow::bail!("NAMU_LOG_FORMAT is `text` or `json`, not {other:?}"),
    };
    let spans = traces
        .as_ref()
        .map(|provider| tracing_opentelemetry::layer().with_tracer(provider.tracer(service)));
    tracing_subscriber::registry()
        .with(filter)
        .with(logs)
        .with(spans)
        .try_init()
        .context("a subscriber is set up already")?;

    if let Some(addr) = var("NAMU_METRICS_ADDR") {
        let addr: SocketAddr = addr.parse().context("NAMU_METRICS_ADDR")?;
        metrics_exporter_prometheus::PrometheusBuilder::new()
            .with_http_listener(addr)
            .install()
            .context("serving metrics")?;
        namu_engine::telemetry::describe();
        tracing::info!(%addr, "serving metrics at /metrics");
    }
    Ok(Telemetry { traces })
}

/// The tracer provider a NAMU process traces with, sending spans to `exporter`: whether a span is
/// sampled is its run's (every worker decides alike), and a run's root span has the run's ids.
pub fn tracer_provider(
    service: &'static str,
    exporter: impl opentelemetry_sdk::trace::SpanExporter + 'static,
) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_id_generator(RunIds::default())
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)))
        .with_resource(Resource::builder().with_service_name(service).build())
        .build()
}

/// Sends a run's root span, from `start` to `end`, if the run is traced. Every worker's task spans
/// are children of it: they know its ids from the run's id (`namu_engine::telemetry::run_trace`),
/// and so does this.
pub fn run_span(
    run: namu_engine::ids::RunId,
    name: String,
    start: SystemTime,
    end: SystemTime,
    attributes: Vec<KeyValue>,
) {
    if !namu_engine::telemetry::sampled_run(run) {
        return;
    }
    let tracer = opentelemetry::global::tracer("namu");
    let builder = tracer
        .span_builder(name)
        .with_start_time(start)
        .with_attributes(attributes);
    // The span is started here, on this thread, with nothing set for it but the run's ids.
    RUN_IDS.set(Some(namu_engine::telemetry::run_trace(run)));
    let mut span = builder.start_with_context(&tracer, &opentelemetry::Context::new());
    RUN_IDS.set(None);
    span.end_with_timestamp(end);
}

thread_local! {
    /// The ids the next root span takes: a run's, while its root span is started.
    static RUN_IDS: Cell<Option<(TraceId, SpanId)>> = const { Cell::new(None) };
}

/// Random ids, but a run's own for its root span (see [`run_span`]).
#[derive(Debug, Default)]
struct RunIds(RandomIdGenerator);

impl IdGenerator for RunIds {
    fn new_trace_id(&self) -> TraceId {
        RUN_IDS
            .get()
            .map_or_else(|| self.0.new_trace_id(), |(trace, _)| trace)
    }

    fn new_span_id(&self) -> SpanId {
        RUN_IDS
            .get()
            .map_or_else(|| self.0.new_span_id(), |(_, span)| span)
    }
}
