//! Async tasks: awaited by the worker rather than run on a thread, for tasks that mostly wait.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use namu::prelude::*;
use namu_engine::LocalCluster;
use serde_json::json;

use crate::common::{json, range, sum, tasks};

/// Waits `ms` (say, for a model server), then doubles `x`.
#[task]
fn remote_double(x: i32, ms: i32) -> Result<i32>;

#[workflow]
fn fan_out_waiting() -> i32 {
    let x = each(range(1, 101));
    let ms = 100;
    let y = remote_double(x, ms);
    sum(y)
}

fn waiting(in_flight: Arc<AtomicUsize>, most: Arc<AtomicUsize>) -> namu_engine::FnTasks {
    tasks().with_async("remote_double", move |(x, ms): (i32, i32)| {
        let (in_flight, most) = (in_flight.clone(), most.clone());
        async move {
            let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(ms as u64)).await;
            in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok::<_, String>(x * 2)
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn waiting_calls_overlap_without_holding_task_slots() {
    let (in_flight, most) = (Arc::default(), Arc::new(AtomicUsize::new(0)));
    let cluster = LocalCluster::uniform(1, Arc::new(waiting(in_flight, most.clone())));
    let started = Instant::now();
    let outcome = cluster
        .run(fan_out_waiting().to_serializable("waiting".into()))
        .await
        .unwrap();
    // 2 * (10 + 20 + .. + 1000)
    assert_eq!(json(&outcome.results), [json!(101000)]);
    // 100 calls of 100 ms each, on one worker: at once, not a slot's worth at a time.
    assert!(most.load(Ordering::SeqCst) > 50, "at most {most:?} at once");
    assert!(
        started.elapsed() < Duration::from_millis(1000),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_async_task_has_a_limit_of_calls_in_flight() {
    let (in_flight, most) = (Arc::default(), Arc::new(AtomicUsize::new(0)));
    let tasks = waiting(in_flight, most.clone()).limit("remote_double", 7);
    let cluster = LocalCluster::uniform(1, Arc::new(tasks));
    let outcome = cluster
        .run(fan_out_waiting().to_serializable("limited".into()))
        .await
        .unwrap();
    assert_eq!(json(&outcome.results), [json!(101000)]);
    assert_eq!(most.load(Ordering::SeqCst), 7);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_async_call_has_its_key_across_awaits() {
    #[task]
    fn whoami(x: i32) -> Result<String>;

    #[workflow]
    fn wf() -> Vec<String> {
        let x = each(range(1, 21));
        let key = whoami(x);
        collect_keys(key)
    }

    #[task]
    fn collect_keys(keys: Vec<String>) -> Result<Vec<String>>;

    let tasks = tasks()
        .with_async("whoami", |_: i32| async {
            let before = namu_engine::call_key().expect("a call has a key");
            // The future may resume on another thread.
            tokio::time::sleep(Duration::from_millis(5)).await;
            let after = namu_engine::call_key().expect("still a key");
            assert_eq!(before, after);
            Ok::<_, String>(after.to_string())
        })
        .with("collect_keys", Ok::<Vec<String>, String>);
    let cluster = LocalCluster::uniform(3, Arc::new(tasks));
    let outcome = cluster
        .run(wf().to_serializable("keys".into()))
        .await
        .unwrap();
    let keys: Vec<String> = outcome.results[0].to().unwrap();
    let distinct: std::collections::HashSet<_> = keys.iter().collect();
    assert_eq!((keys.len(), distinct.len()), (20, 20));
}

#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_fail_and_are_attempted_like_any_other() {
    #[task]
    fn flaky(x: i32) -> Result<i32>;

    #[workflow]
    fn wf() -> i32 {
        let x = each(range(1, 5));
        let y = match attempt(flaky(x)) {
            Ok(v) => v,
            Err(_) => 0,
        };
        sum(y)
    }

    let tasks = tasks().with_async("flaky", |x: i32| async move {
        match x {
            20 => Err("timed out".to_string()),
            x => Ok(x),
        }
    });
    let cluster = LocalCluster::uniform(3, Arc::new(tasks));
    let outcome = cluster
        .run(wf().to_serializable("flaky".into()))
        .await
        .unwrap();
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
    assert_eq!(json(&outcome.results), [json!(80)]);
}
