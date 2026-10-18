//! Workers killed in the middle of a run: the run must end as if nothing happened.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use namu::prelude::*;
use namu_core::ir::Workflow;
use namu_engine::{FnTasks, LocalCluster, Tasks};
use serde_json::json;

use crate::common::{range, sum};

#[task]
fn slow(x: i32) -> Result<i32>;

#[task]
fn mark(x: i32) -> Result<i32>;

/// Each item takes a while, so a run is still in flight when a worker dies.
fn tasks(marks: Arc<AtomicUsize>) -> Arc<dyn Tasks> {
    Arc::new(
        FnTasks::new()
            .with("range", |(start, end): (i32, i32)| {
                Ok::<_, String>((start..end).map(|x| x * 10).collect::<Vec<_>>())
            })
            .with("slow", |x: i32| {
                std::thread::sleep(Duration::from_millis(30));
                Ok::<_, String>(x * 2)
            })
            .with("mark", move |x: i32| {
                marks.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>(x)
            })
            .with("sum", |xs: Vec<i32>| {
                Ok::<_, String>(xs.iter().sum::<i32>())
            })
            .checkpoint("mark"),
    )
}

#[workflow]
fn doubled_sum() -> i32 {
    let xs = range(1, 25);
    let ys = slow(xs);
    let zs = slow(ys);
    sum(zs)
}

fn workflow() -> Workflow {
    doubled_sum().to_serializable("doubled_sum".into())
}

/// (10 + .. + 240) * 4
const DOUBLED_SUM: i32 = 3000 * 4;

/// Runs `workflow` on `n` workers and kills `victims`, one by one, while it runs.
async fn run_with_crashes(
    n: usize,
    victims: &[u64],
    workflow: Workflow,
) -> namu_engine::RunOutcome {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let marks = Arc::new(AtomicUsize::new(0));
    let cluster = Arc::new(LocalCluster::new(vec![tasks(marks); n]));
    let run = tokio::spawn({
        let cluster = cluster.clone();
        async move { cluster.run(workflow).await }
    });
    for &victim in victims {
        tokio::time::sleep(Duration::from_millis(40)).await;
        cluster.kill(victim).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("the run finished")
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_survives_a_worker_crash() {
    let outcome = run_with_crashes(4, &[3], workflow()).await;
    assert_eq!(outcome.results, [json!(DOUBLED_SUM)]);
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_survives_its_owner_crashing() {
    // The first run on a fresh cluster is owned by worker 1.
    let outcome = run_with_crashes(4, &[1], workflow()).await;
    assert_eq!(outcome.results, [json!(DOUBLED_SUM)]);
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_survives_a_successor_crashing_too() {
    // Worker 1 takes over from 2, then dies itself: worker 3 takes over both.
    let outcome = run_with_crashes(5, &[2, 1], workflow()).await;
    assert_eq!(outcome.results, [json!(DOUBLED_SUM)]);
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
}

#[tokio::test(flavor = "multi_thread")]
async fn checkpoint_tasks_are_not_repeated() {
    #[workflow]
    fn wf() -> i32 {
        let xs = range(1, 9);
        let marked = mark(xs);
        let ys = slow(marked);
        let zs = slow(ys);
        sum(zs)
    }

    let marks = Arc::new(AtomicUsize::new(0));
    let cluster = Arc::new(LocalCluster::new(vec![tasks(marks.clone()); 3]));
    let run = tokio::spawn({
        let cluster = cluster.clone();
        async move { cluster.run(wf().to_serializable("marked".into())).await }
    });
    // Every `mark` has run (and been recorded past) long before the slow tasks are done.
    tokio::time::sleep(Duration::from_millis(40)).await;
    cluster.kill(2).unwrap();
    let outcome = run.await.unwrap().unwrap();
    assert_eq!(outcome.results, [json!(360 * 4)]);
    assert_eq!(marks.load(Ordering::SeqCst), 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_survives_a_crash_at_any_moment() {
    // From before the fan-out to the final collect, for an ordinary worker and for the owner.
    for victim in [2, 1] {
        for delay in [0, 2, 5, 10, 20, 35, 50, 65, 80, 100] {
            let marks = Arc::new(AtomicUsize::new(0));
            let cluster = Arc::new(LocalCluster::new(vec![tasks(marks); 3]));
            let run = tokio::spawn({
                let cluster = cluster.clone();
                async move { cluster.run(workflow()).await }
            });
            tokio::time::sleep(Duration::from_millis(delay)).await;
            cluster.kill(victim).unwrap();
            let outcome = tokio::time::timeout(Duration::from_secs(20), run)
                .await
                .unwrap_or_else(|_| panic!("stuck after killing {victim} at {delay} ms"))
                .unwrap()
                .unwrap();
            assert_eq!(
                outcome.results,
                [json!(DOUBLED_SUM)],
                "killing {victim} at {delay} ms"
            );
        }
    }
}
