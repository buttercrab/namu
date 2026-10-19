//! Stream tasks: items fan out as they are yielded, with backpressure, and survive a crash of the
//! worker running the stream.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use namu::prelude::*;
use namu_engine::{FnTasks, LocalCluster, RunOutcome, Tasks};
use serde_json::json;

use crate::common::{add, double, json, sum};

/// `0, 1, .., n - 1`, one item every `delay_ms`.
#[task(stream)]
fn numbers(n: i32, delay_ms: i32) -> Result<impl Iterator<Item = Result<i32>>>;

/// Like `numbers`, but fails after `n` items.
#[task(stream)]
fn numbers_then_fail(n: i32) -> Result<impl Iterator<Item = Result<i32>>>;

#[task]
fn count(xs: Vec<i32>) -> Result<i32>;

#[task]
fn listing(xs: Vec<i32>) -> Result<Vec<i32>>;

/// Records how many calls are in flight at once.
#[task]
fn tracked(x: i32) -> Result<i32>;

#[derive(Default)]
struct Probe {
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    processed: AtomicUsize,
    pipelined: AtomicBool,
}

fn tasks(probe: Arc<Probe>) -> FnTasks {
    let pulled = probe.clone();
    let tracked = probe;
    FnTasks::new()
        .with_stream("numbers", move |(n, delay): (i32, u64)| {
            let probe = pulled.clone();
            Ok::<_, String>((0..n).map(move |i| {
                std::thread::sleep(Duration::from_millis(delay));
                if i == n - 1 && probe.processed.load(Ordering::SeqCst) > 0 {
                    probe.pipelined.store(true, Ordering::SeqCst);
                }
                Ok::<_, String>(i)
            }))
        })
        .with_stream("numbers_then_fail", |n: i32| {
            Ok::<_, String>((0..=n).map(move |i| match i == n {
                true => Err("the source broke".to_string()),
                false => Ok(i),
            }))
        })
        .with("add", |(a, b): (i32, i32)| Ok::<_, String>(a + b))
        .with("double", |x: i32| Ok::<_, String>(x * 2))
        .with("sum", |xs: Vec<i32>| {
            Ok::<_, String>(xs.iter().sum::<i32>())
        })
        .with("count", |xs: Vec<i32>| Ok::<_, String>(xs.len() as i32))
        .with("listing", Ok::<Vec<i32>, String>)
        .with("tracked", move |x: i32| {
            let now = tracked.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            tracked.max_in_flight.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(5));
            tracked.in_flight.fetch_sub(1, Ordering::SeqCst);
            tracked.processed.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(x)
        })
}

async fn run_on(cluster: &LocalCluster, workflow: namu_core::ir::Workflow) -> RunOutcome {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    tokio::time::timeout(Duration::from_secs(20), cluster.run(workflow))
        .await
        .expect("the run finished")
        .unwrap()
}

fn cluster(n: usize, tasks: FnTasks) -> LocalCluster {
    LocalCluster::uniform(n, Arc::new(tasks))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_fans_out_and_collects_in_order() {
    #[workflow]
    fn wf() -> Vec<i32> {
        let n = 6;
        let delay = 0;
        let xs = numbers(n, delay);
        let ys = double(xs);
        listing(ys)
    }

    let outcome = run_on(
        &cluster(3, tasks(Arc::default())),
        wf().to_serializable("s".into()),
    )
    .await;
    assert_eq!(json(&outcome.results), [json!([0, 2, 4, 6, 8, 10])]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_used_whole_is_read_into_a_list() {
    #[workflow]
    fn wf() -> i32 {
        let n = 5;
        let delay = 0;
        let xs = numbers(n, delay);
        let total = sum(xs);
        let size = count(xs);
        add(total, size)
    }

    let outcome = run_on(
        &cluster(2, tasks(Arc::default())),
        wf().to_serializable("s".into()),
    )
    .await;
    // (0 + .. + 4) + 5
    assert_eq!(json(&outcome.results), [json!(15)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn items_are_processed_before_the_stream_ends() {
    #[workflow]
    fn wf() -> i32 {
        let n = 8;
        let delay = 15;
        let xs = numbers(n, delay);
        let ys = tracked(xs);
        sum(ys)
    }

    let probe = Arc::new(Probe::default());
    let outcome = run_on(
        &cluster(2, tasks(probe.clone())),
        wf().to_serializable("s".into()),
    )
    .await;
    assert_eq!(json(&outcome.results), [json!(28)]);
    assert!(
        probe.pipelined.load(Ordering::SeqCst),
        "items waited for the end of the stream"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_never_has_more_than_its_window_in_flight() {
    #[workflow]
    fn wf() -> i32 {
        let n = 40;
        let delay = 0;
        let xs = numbers(n, delay);
        let ys = tracked(xs);
        sum(ys)
    }

    let probe = Arc::new(Probe::default());
    let tasks = tasks(probe.clone()).window("numbers", 3);
    let outcome = run_on(&cluster(4, tasks), wf().to_serializable("s".into())).await;
    assert_eq!(json(&outcome.results), [json!(780)]);
    let max = probe.max_in_flight.load(Ordering::SeqCst);
    assert!((1..=3).contains(&max), "{max} items in flight");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_stream_keeps_what_it_yielded() {
    #[workflow]
    fn wf() -> i32 {
        let n = 4;
        let xs = numbers_then_fail(n);
        let ys = double(xs);
        sum(ys)
    }

    let outcome = run_on(
        &cluster(2, tasks(Arc::default())),
        wf().to_serializable("s".into()),
    )
    .await;
    assert_eq!(json(&outcome.results), [json!(12)]);
    assert_eq!(outcome.failures, ["numbers_then_fail: the source broke"]);
}

#[workflow]
fn slow_stream() -> i32 {
    let n = 30;
    let delay = 10;
    let xs = numbers(n, delay);
    let ys = double(xs);
    sum(ys)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_resumes_after_its_worker_dies() {
    // The first run is owned by worker 1, which also runs the stream; kill it mid-stream.
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let cluster = Arc::new(cluster(3, tasks(Arc::default())));
    let run = tokio::spawn({
        let cluster = cluster.clone();
        async move { cluster.run(slow_stream().to_serializable("s".into())).await }
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    cluster.kill(1).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("the run finished")
        .unwrap()
        .unwrap();
    assert_eq!(json(&outcome.results), [json!(870)]);
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_moves_on_when_its_worker_drains() {
    // Worker 1 owns the first run and feeds its stream; it drains mid-stream, and its successor
    // resumes the stream where it stopped: every item is processed once.
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let probe = Arc::new(Probe::default());
    let cluster = Arc::new(cluster(3, tasks(probe.clone())));
    let run = tokio::spawn({
        let cluster = cluster.clone();
        async move {
            cluster
                .run(tracked_stream().to_serializable("s".into()))
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    tokio::time::timeout(Duration::from_secs(20), cluster.drain(1))
        .await
        .expect("the worker drained")
        .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("the run finished")
        .unwrap()
        .unwrap();
    assert_eq!(json(&outcome.results), [json!(435)]);
    assert_eq!(probe.processed.load(Ordering::SeqCst), 30);
}

#[workflow]
fn tracked_stream() -> i32 {
    let n = 30;
    let delay = 10;
    let xs = numbers(n, delay);
    let ys = tracked(xs);
    sum(ys)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_restarts_where_its_task_is_served() {
    // Worker 2 succeeds worker 1 but cannot run the stream: the stream starts again on worker 3.
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let all: Arc<dyn Tasks> = Arc::new(tasks(Arc::default()));
    let without_stream: Arc<dyn Tasks> = Arc::new(
        FnTasks::new()
            .with("double", |x: i32| Ok::<_, String>(x * 2))
            .with("sum", |xs: Vec<i32>| {
                Ok::<_, String>(xs.iter().sum::<i32>())
            }),
    );
    let cluster = Arc::new(LocalCluster::new(vec![all.clone(), without_stream, all]));
    let run = tokio::spawn({
        let cluster = cluster.clone();
        async move { cluster.run(slow_stream().to_serializable("s".into())).await }
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    cluster.kill(1).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("the run finished")
        .unwrap()
        .unwrap();
    assert_eq!(json(&outcome.results), [json!(870)]);
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
}
