//! A run is one trace across its workers, and its calls are counted.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use namu_core::ir::{Action, Call, Literal, Next, Operation, Workflow};
use namu_engine::{FnTasks, LocalCluster, Tasks};
use opentelemetry::trace::{SpanId, TraceId, TracerProvider as _};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// `double(double(1))`.
fn twice() -> Workflow {
    let call = |input, output| Call {
        task_id: "double".into(),
        inputs: vec![input],
        outputs: vec![output],
        catch: None,
    };
    let literal = Literal {
        output: 0,
        value: "1".into(),
    };
    Workflow::new(
        "twice".into(),
        vec![
            Operation::new(
                vec![literal],
                Vec::new(),
                Some(Action::Call(call(0, 1))),
                Next::jump(1),
            ),
            Operation::new(
                Vec::new(),
                Vec::new(),
                Some(Action::Call(call(1, 2))),
                Next::return_value(2),
            ),
        ],
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_is_one_trace_and_its_calls_are_counted() {
    let recorder = DebuggingRecorder::new();
    let metrics = recorder.snapshotter();
    recorder.install().unwrap();
    let spans = InMemorySpanExporter::default();
    let provider = namu_telemetry::tracer_provider("test", spans.clone());
    opentelemetry::global::set_tracer_provider(provider.clone());
    tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")))
        .init();

    let tasks: Arc<dyn Tasks> =
        Arc::new(FnTasks::new().with("double", |x: i32| Ok::<_, String>(x * 2)));
    let cluster = LocalCluster::new(vec![tasks.clone(), tasks]);
    cluster.run(twice()).await.unwrap();
    let run = *cluster.recorded().keys().next().expect("the run reported");
    let now = SystemTime::now();
    namu_telemetry::run_span(
        run,
        "run twice".into(),
        now - Duration::from_secs(1),
        now,
        Vec::new(),
    );
    provider.force_flush().unwrap();

    // Every call's span, on whichever worker, is a child of the run's root span, in the run's
    // trace; the root span the control plane sends has those very ids.
    let (trace, root) = namu_engine::telemetry::run_trace(run);
    assert_eq!(trace, TraceId::from_bytes(run.into_bytes()));
    let finished = spans.get_finished_spans().unwrap();
    let calls: Vec<_> = finished.iter().filter(|s| s.name == "task").collect();
    assert_eq!(calls.len(), 2, "{finished:?}");
    for call in &calls {
        assert_eq!(call.span_context.trace_id(), trace);
        assert_eq!(call.parent_span_id, root);
    }
    let roots: Vec<_> = finished.iter().filter(|s| s.name == "run twice").collect();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].span_context.trace_id(), trace);
    assert_eq!(roots[0].span_context.span_id(), root);
    assert_eq!(roots[0].parent_span_id, SpanId::INVALID);

    let snapshot = metrics.snapshot().into_vec();
    let calls_ok = snapshot.iter().find_map(|(key, _, _, value)| {
        let key = key.key();
        let labels: Vec<_> = key.labels().map(|l| (l.key(), l.value())).collect();
        (key.name() == "namu_task_calls_total"
            && labels.contains(&("task", "double"))
            && labels.contains(&("outcome", "ok")))
        .then_some(value)
    });
    assert_eq!(calls_ok, Some(&DebugValue::Counter(2)));
    let durations = snapshot.iter().find_map(|(key, _, _, value)| match value {
        DebugValue::Histogram(samples) if key.key().name() == "namu_task_call_seconds" => {
            Some(samples.len())
        }
        _ => None,
    });
    assert_eq!(durations, Some(2));
}
