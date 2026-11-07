//! A run's results reach whoever started it as they are recorded, not only when it ends: an
//! unbounded run has results too. Each is sent once its owner has logged it, and again by a
//! successor that takes the run over, and counts once.

mod common;

use std::sync::Arc;
use std::time::Duration;

use namu::prelude::*;
use namu_engine::message::Report;
use namu_engine::{FnTasks, LocalCluster, RunOutcome, Tasks};

use crate::common::{json, range};

/// `0, 1, .., n - 1`, one every `delay_ms`, then a pause of `pause_ms` before the stream ends.
#[task(stream)]
fn slow_numbers(n: i32, delay_ms: i32, pause_ms: i32) -> Result<impl Iterator<Item = Result<i32>>>;

#[task]
fn slow(x: i32) -> Result<i32>;

fn tasks() -> Arc<dyn Tasks> {
    Arc::new(
        FnTasks::new()
            .with("range", |(start, end): (i32, i32)| {
                Ok::<_, String>((start..end).map(|x| x * 10).collect::<Vec<_>>())
            })
            .with("slow", |x: i32| {
                std::thread::sleep(Duration::from_millis(20));
                match x {
                    70 => Err("seventy".to_string()),
                    x => Ok(x + 1),
                }
            })
            .with_stream("slow_numbers", |(n, delay, pause): (i32, u64, u64)| {
                let end = std::iter::from_fn(move || {
                    std::thread::sleep(Duration::from_millis(pause));
                    None
                });
                let items = (0..n).map(move |i| {
                    std::thread::sleep(Duration::from_millis(delay));
                    Ok::<_, String>(i)
                });
                Ok::<_, String>(items.chain(end))
            }),
    )
}

/// Every branch returns its own result: there is no collect.
#[workflow]
fn per_item() -> i32 {
    let x = each(range(0, 12));
    slow(x)
}

/// What a run's reports say, sorted, to compare with its outcome.
fn sorted(reports: &[Report]) -> (Vec<String>, Vec<String>) {
    let mut results = Vec::new();
    let mut failures = Vec::new();
    for report in reports {
        match report {
            Report::Result(value) => results.push(json(std::slice::from_ref(value))[0].to_string()),
            Report::Failure(failure) => failures.push(failure.clone()),
        }
    }
    results.sort();
    failures.sort();
    (results, failures)
}

fn outcome_sorted(outcome: &RunOutcome) -> (Vec<String>, Vec<String>) {
    let mut results: Vec<String> = json(&outcome.results)
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut failures = outcome.failures.clone();
    results.sort();
    failures.sort();
    (results, failures)
}

#[tokio::test(flavor = "multi_thread")]
async fn every_report_is_sent_once_as_it_is_recorded() {
    for workers in [1, 3] {
        let cluster = LocalCluster::new(vec![tasks(); workers]);
        let outcome = cluster
            .run(per_item().to_serializable("per_item".into()))
            .await
            .unwrap();
        // 0, 10, .. 110, plus one each, but 70 fails.
        assert_eq!(outcome.results.len(), 11, "{workers} workers");
        assert_eq!(outcome.failures.len(), 1, "{workers} workers");
        let recorded = cluster.recorded();
        let [(_, reports)] = &recorded.into_iter().collect::<Vec<_>>()[..] else {
            panic!("one run");
        };
        assert_eq!(
            sorted(reports),
            outcome_sorted(&outcome),
            "{workers} workers"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_successor_sends_the_reports_it_finds_in_the_log() {
    for delay in [60, 120, 180] {
        let cluster = Arc::new(LocalCluster::new(vec![tasks(); 3]));
        let run = tokio::spawn({
            let cluster = cluster.clone();
            async move {
                cluster
                    .run(per_item().to_serializable("per_item".into()))
                    .await
            }
        });
        tokio::time::sleep(Duration::from_millis(delay)).await;
        // The first run on a fresh cluster is owned by worker 1.
        cluster.kill(1).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(20), run)
            .await
            .unwrap_or_else(|_| panic!("stuck after killing the owner at {delay} ms"))
            .unwrap()
            .unwrap();
        let recorded = cluster.recorded();
        let reports: Vec<Report> = recorded.into_values().flatten().collect();
        assert_eq!(
            sorted(&reports),
            outcome_sorted(&outcome),
            "killing the owner at {delay} ms"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn results_arrive_while_the_run_goes_on() {
    #[workflow]
    fn streamed() -> i32 {
        let n = 5;
        let delay = 5;
        let pause = 1000;
        let x = each(slow_numbers(n, delay, pause));
        slow(x)
    }

    let cluster = Arc::new(LocalCluster::new(vec![tasks(); 2]));
    let run = tokio::spawn({
        let cluster = cluster.clone();
        async move {
            cluster
                .run(streamed().to_serializable("streamed".into()))
                .await
        }
    });
    // Every item is done long before the stream ends, a second later.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(800);
    loop {
        let reports: usize = cluster.recorded().values().map(Vec::len).sum();
        if reports == 5 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{reports} of 5 reports so far"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!run.is_finished(), "the stream has not ended");
    let outcome = run.await.unwrap().unwrap();
    assert_eq!(json(&outcome.results).len(), 5);
    assert_eq!(
        outcome_sorted(&outcome).0,
        ["1", "2", "3", "4", "5"].map(String::from)
    );
}

/// A report leaves the log once the run's starter stored it and its branch ended in its scope:
/// the log holds the reports in flight, not every result, however many a run makes.
#[tokio::test(flavor = "multi_thread")]
async fn the_log_holds_reports_in_flight_not_every_result() {
    #[workflow]
    fn many() -> i32 {
        let n = 400;
        let delay = 1;
        let pause = 0;
        each(slow_numbers(n, delay, pause))
    }

    for workers in [1, 3] {
        let cluster = LocalCluster::new(vec![tasks(); workers]);
        let outcome = cluster
            .run(many().to_serializable("many".into()))
            .await
            .unwrap();
        assert_eq!(outcome.results.len(), 400, "{workers} workers");
        let peak = cluster.log().peak_reports();
        println!("{workers} workers: at most {peak} of 400 reports in the log at once");
        assert!(
            peak < 40,
            "{workers} workers: {peak} reports in the log at once"
        );
    }
}

/// A scope's ends leave the log once settled: a stream that never ends holds the ends in flight,
/// not every one.
#[tokio::test(flavor = "multi_thread")]
async fn the_log_holds_ends_in_flight_not_every_end() {
    #[workflow]
    fn many() -> i32 {
        let n = 2000;
        let delay = 0;
        let pause = 0;
        each(slow_numbers(n, delay, pause))
    }

    let cluster = LocalCluster::new(vec![tasks(); 3]);
    let outcome = cluster
        .run(many().to_serializable("many".into()))
        .await
        .unwrap();
    assert_eq!(outcome.results.len(), 2000);
    let peak = cluster.log().peak_ends();
    assert!(
        peak < 1000,
        "a scope's log held {peak} of 2000 ends at once"
    );
}

/// A successor takes a settled scope up from where it was settled: nothing before it is lost or
/// done again.
#[tokio::test(flavor = "multi_thread")]
async fn a_settled_scope_survives_its_owner_dying() {
    #[workflow]
    fn many() -> i32 {
        let n = 2000;
        let delay = 1;
        let pause = 0;
        each(slow_numbers(n, delay, pause))
    }

    for delay in [600, 1500] {
        let cluster = Arc::new(LocalCluster::new(vec![tasks(); 3]));
        let run = tokio::spawn({
            let cluster = cluster.clone();
            async move { cluster.run(many().to_serializable("many".into())).await }
        });
        tokio::time::sleep(Duration::from_millis(delay)).await;
        // The first run on a fresh cluster is owned by worker 1, which owns its stream's scope.
        cluster.kill(1).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(30), run)
            .await
            .unwrap_or_else(|_| panic!("stuck after killing the owner at {delay} ms"))
            .unwrap()
            .unwrap();
        let mut got: Vec<i64> = json(&outcome.results)
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        got.sort_unstable();
        assert_eq!(
            got,
            (0..2000).collect::<Vec<_>>(),
            "killing the owner at {delay} ms"
        );
    }
}

/// An ordered fold over a long stream, settled as it goes, and its owner killed in the middle: the
/// fold is exactly the one over every item, in order, each once.
#[tokio::test(flavor = "multi_thread")]
async fn a_long_fold_survives_its_owner_dying() {
    #[task(reduce)]
    fn join_in(list: String, x: i32) -> Result<String>;

    #[workflow]
    fn joined() -> String {
        let n = 1000;
        let delay = 1;
        let pause = 0;
        let x = each(slow_numbers(n, delay, pause));
        join_in("", x)
    }

    let tasks: Arc<dyn Tasks> = Arc::new(
        FnTasks::new()
            .with_stream("slow_numbers", |(n, delay, _): (i32, u64, u64)| {
                Ok::<_, String>((0..n).map(move |i| {
                    std::thread::sleep(Duration::from_millis(delay));
                    Ok::<_, String>(i)
                }))
            })
            .with("join_in", |(list, x): (String, i32)| {
                Ok::<_, String>(match list.is_empty() {
                    true => x.to_string(),
                    false => format!("{list},{x}"),
                })
            }),
    );
    let cluster = Arc::new(LocalCluster::new(vec![tasks; 3]));
    let run = tokio::spawn({
        let cluster = cluster.clone();
        async move { cluster.run(joined().to_serializable("joined".into())).await }
    });
    tokio::time::sleep(Duration::from_millis(700)).await;
    cluster.kill(1).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .expect("the run finished")
        .unwrap()
        .unwrap();
    let want: Vec<String> = (0..1000).map(|i| i.to_string()).collect();
    assert_eq!(json(&outcome.results), [serde_json::json!(want.join(","))]);
}
