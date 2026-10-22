//! Workers killed in the middle of a run: the run must end as if nothing happened.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use namu::prelude::*;
use namu_core::ir::Workflow;
use namu_engine::{FnTasks, LocalCluster, Tasks};
use serde_json::json;

use crate::common::{json, range, sum};

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
    assert_eq!(json(&outcome.results), [json!(DOUBLED_SUM)]);
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_survives_its_owner_crashing() {
    // The first run on a fresh cluster is owned by worker 1.
    let outcome = run_with_crashes(4, &[1], workflow()).await;
    assert_eq!(json(&outcome.results), [json!(DOUBLED_SUM)]);
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_survives_a_successor_crashing_too() {
    // Worker 1 takes over from 2, then dies itself: worker 3 takes over both.
    let outcome = run_with_crashes(5, &[2, 1], workflow()).await;
    assert_eq!(json(&outcome.results), [json!(DOUBLED_SUM)]);
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
    assert_eq!(json(&outcome.results), [json!(360 * 4)]);
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
                json(&outcome.results),
                [json!(DOUBLED_SUM)],
                "killing {victim} at {delay} ms"
            );
        }
    }
}

/// Like `tasks`, counting every call of `slow`.
fn counted(calls: Arc<AtomicUsize>) -> Arc<dyn Tasks> {
    Arc::new(
        FnTasks::new()
            .with("range", |(start, end): (i32, i32)| {
                Ok::<_, String>((start..end).map(|x| x * 10).collect::<Vec<_>>())
            })
            .with("slow", move |x: i32| {
                calls.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(30));
                Ok::<_, String>(x * 2)
            })
            .with("sum", |xs: Vec<i32>| {
                Ok::<_, String>(xs.iter().sum::<i32>())
            }),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drained_worker_hands_everything_on_and_nothing_runs_twice() {
    // An ordinary worker, then the run's owner (which owns the scope and the run).
    for leaving in [2, 1] {
        let calls = Arc::new(AtomicUsize::new(0));
        let cluster = Arc::new(LocalCluster::new(vec![counted(calls.clone()); 3]));
        let run = tokio::spawn({
            let cluster = cluster.clone();
            async move { cluster.run(workflow()).await }
        });
        tokio::time::sleep(Duration::from_millis(40)).await;
        tokio::time::timeout(Duration::from_secs(20), cluster.drain(leaving))
            .await
            .expect("the worker drained")
            .unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(20), run)
            .await
            .expect("the run finished")
            .unwrap()
            .unwrap();
        assert_eq!(
            json(&outcome.results),
            [json!(DOUBLED_SUM)],
            "draining {leaving}"
        );
        // 24 items, two calls each: a crash would repeat some; a drain repeats none.
        assert_eq!(calls.load(Ordering::SeqCst), 48, "draining {leaving}");
    }
}

/// Like `tasks`, recording the key of every call of `slow`.
fn keyed(keys: Arc<std::sync::Mutex<Vec<String>>>) -> Arc<dyn Tasks> {
    Arc::new(
        FnTasks::new()
            .with("range", |(start, end): (i32, i32)| {
                Ok::<_, String>((start..end).map(|x| x * 10).collect::<Vec<_>>())
            })
            .with("slow", move |x: i32| {
                let key = namu_engine::call_key().expect("a call has a key");
                keys.lock().unwrap().push(key.to_string());
                std::thread::sleep(Duration::from_millis(30));
                Ok::<_, String>(x * 2)
            })
            .with("sum", |xs: Vec<i32>| {
                Ok::<_, String>(xs.iter().sum::<i32>())
            }),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_run_again_after_a_crash_keeps_its_key() {
    for victim in [2, 1] {
        let keys = Arc::new(std::sync::Mutex::new(Vec::new()));
        let cluster = Arc::new(LocalCluster::new(vec![keyed(keys.clone()); 3]));
        let run = tokio::spawn({
            let cluster = cluster.clone();
            async move { cluster.run(workflow()).await }
        });
        tokio::time::sleep(Duration::from_millis(40)).await;
        cluster.kill(victim).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(20), run)
            .await
            .expect("the run finished")
            .unwrap()
            .unwrap();
        assert_eq!(json(&outcome.results), [json!(DOUBLED_SUM)]);
        let keys = keys.lock().unwrap();
        let distinct: std::collections::HashSet<_> = keys.iter().collect();
        // 24 items, two calls each. The crash runs some again, under the keys they had.
        assert_eq!(distinct.len(), 48, "killing {victim}");
        assert!(keys.len() > 48, "killing {victim} repeated no call");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fan_out_done_again_names_its_calls_as_before() {
    #[task]
    fn deliver(x: i32) -> Result<i32>;

    #[workflow]
    fn wf() -> i32 {
        let xs = range(1, 25);
        let ys = deliver(xs);
        sum(ys)
    }

    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let keys = Arc::new(std::sync::Mutex::new(Vec::new()));
    let tasks: Arc<dyn Tasks> = Arc::new(
        FnTasks::new()
            .with("range", |(start, end): (i32, i32)| {
                Ok::<_, String>((start..end).map(|x| x * 10).collect::<Vec<_>>())
            })
            .with("deliver", {
                let keys = keys.clone();
                move |x: i32| {
                    let key = namu_engine::call_key().expect("a call has a key");
                    keys.lock().unwrap().push(key.to_string());
                    std::thread::sleep(Duration::from_millis(20));
                    Ok::<_, String>(x)
                }
            })
            .with("sum", |xs: Vec<i32>| {
                Ok::<_, String>(xs.iter().sum::<i32>())
            }),
    );
    // What a worker logs takes 100 ms to be durable. The run's owner fans out and dies before its
    // scope is: its successor fans out again, under a new scope id, while the first items' branches
    // are already delivering elsewhere.
    let cluster = Arc::new(LocalCluster::with_log_delay(
        vec![tasks; 3],
        Duration::from_millis(100),
    ));
    let run = tokio::spawn({
        let cluster = cluster.clone();
        async move { cluster.run(workflow_of(wf())).await }
    });
    tokio::time::sleep(Duration::from_millis(130)).await;
    cluster.kill(1).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("the run finished")
        .unwrap()
        .unwrap();
    assert_eq!(json(&outcome.results), [json!(3000)]);
    let keys = keys.lock().unwrap();
    let distinct: std::collections::HashSet<_> = keys.iter().collect();
    assert!(
        keys.len() > 24,
        "nothing was delivered twice: {}",
        keys.len()
    );
    assert_eq!(
        distinct.len(),
        24,
        "{} keys for 24 deliveries",
        distinct.len()
    );
}

fn workflow_of<T>(graph: namu::__macro_exports::Graph<T>) -> Workflow {
    graph.to_serializable("fan_out_again".into())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reduce_survives_its_owner_dying_mid_fold() {
    #[task(reduce)]
    fn fold_in(list: String, x: i32) -> Result<String>;

    #[workflow]
    fn wf() -> String {
        let xs = range(1, 25);
        let ys = slow(xs);
        fold_in("", ys)
    }

    let folds = Arc::new(std::sync::Mutex::new(Vec::new()));
    let tasks: Arc<dyn Tasks> = Arc::new(
        FnTasks::new()
            .with("range", |(start, end): (i32, i32)| {
                Ok::<_, String>((start..end).map(|x| x * 10).collect::<Vec<_>>())
            })
            .with("slow", |x: i32| {
                std::thread::sleep(Duration::from_millis(30));
                Ok::<_, String>(x * 2)
            })
            .with("fold_in", {
                let folds = folds.clone();
                move |(list, x): (String, i32)| {
                    let key = namu_engine::call_key().expect("a fold step has a key");
                    folds.lock().unwrap().push(key.to_string());
                    // Each step takes a while: the owner dies in the middle of the fold.
                    std::thread::sleep(Duration::from_millis(20));
                    Ok::<_, String>(match list.is_empty() {
                        true => x.to_string(),
                        false => format!("{list},{x}"),
                    })
                }
            }),
    );
    // What a worker logs takes 30 ms to be durable, as on Redis: the owner dies with fold steps
    // done that its successor does not know of, and does them again.
    let cluster = Arc::new(LocalCluster::with_log_delay(
        vec![tasks; 3],
        Duration::from_millis(30),
    ));
    let run = tokio::spawn({
        let cluster = cluster.clone();
        async move { cluster.run(workflow_of(wf())).await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let folded_before = folds.lock().unwrap().len();
    cluster.kill(1).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("the run finished")
        .unwrap()
        .unwrap();
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
    assert!(
        (1..24).contains(&folded_before),
        "the owner died with {folded_before} of 24 steps done, not in the middle"
    );
    let expected: Vec<String> = (1..25).map(|x| (x * 20).to_string()).collect();
    assert_eq!(json(&outcome.results), [json!(expected.join(","))]);
    // Every step done again had the key it had the first time.
    let folds = folds.lock().unwrap();
    let distinct: std::collections::HashSet<_> = folds.iter().collect();
    assert_eq!(
        distinct.len(),
        24,
        "{} keys for 24 fold steps",
        distinct.len()
    );
    assert!(folds.len() > 24, "no step was done again");
}
