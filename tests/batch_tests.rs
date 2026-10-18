//! Batch tasks: items from many branches (and runs) share steps of a session, and each branch
//! gets its own result as soon as it is ready.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use namu::prelude::*;
use namu_core::ir::Workflow;
use namu_engine::tasks::{BatchPolicy, Continuous};
use namu_engine::{FnTasks, LocalCluster, RunOutcome, Tasks};
use serde_json::json;

use crate::common::sum;

/// `[0, 1, .., n - 1]`.
#[task]
fn items(n: i32) -> Result<Vec<i32>>;

/// `x * 10`, in batches; 13 fails on its own.
#[task(batch)]
fn embed(xs: Vec<i32>) -> Vec<Result<i32>>;

/// Fails as a whole.
#[task(batch)]
fn broken(xs: Vec<i32>) -> Vec<Result<i32>>;

/// Continuous batching: item `x` takes `x` steps and returns `x`.
#[task(batch)]
fn decode(xs: Vec<i32>) -> Vec<Result<i32>>;

/// What the batch tasks saw.
#[derive(Default)]
struct Log {
    /// The items admitted by each step.
    steps: Mutex<Vec<Vec<i32>>>,
    /// Items in the order they finished.
    finished: Mutex<Vec<i32>>,
    max_active: Mutex<usize>,
}

impl Log {
    fn steps(&self) -> Vec<Vec<i32>> {
        self.steps.lock().unwrap().clone()
    }
}

/// Each item needs as many steps as its value; it leaves the moment it is done.
struct Decoder {
    log: Arc<Log>,
    active: HashMap<u64, (i32, i32)>,
}

impl Continuous for Decoder {
    type In = i32;
    type Out = i32;

    fn step(&mut self, admit: Vec<(u64, i32)>) -> Vec<(u64, Result<i32, String>)> {
        self.log
            .steps
            .lock()
            .unwrap()
            .push(admit.iter().map(|(_, x)| *x).collect());
        for (id, x) in admit {
            self.active.insert(id, (x, x));
        }
        let mut max = self.log.max_active.lock().unwrap();
        *max = (*max).max(self.active.len());
        std::thread::sleep(Duration::from_millis(5));
        let mut done = Vec::new();
        self.active.retain(|&id, (x, left)| {
            *left -= 1;
            if *left > 0 {
                return true;
            }
            done.push((id, Ok(*x)));
            false
        });
        let mut finished = self.log.finished.lock().unwrap();
        finished.extend(done.iter().map(|(_, x)| *x.as_ref().unwrap()));
        done
    }
}

fn tasks(log: Arc<Log>) -> FnTasks {
    let (embedded, decoded) = (log.clone(), log);
    FnTasks::new()
        .with("items", |n: i32| {
            Ok::<_, String>((0..n).collect::<Vec<_>>())
        })
        .with("sum", |xs: Vec<i32>| {
            Ok::<_, String>(xs.iter().sum::<i32>())
        })
        .with_batch("embed", move |xs: Vec<i32>| {
            embedded.steps.lock().unwrap().push(xs.clone());
            std::thread::sleep(Duration::from_millis(20));
            Ok::<_, String>(
                xs.into_iter()
                    .map(|x| match x {
                        13 => Err("unlucky".to_string()),
                        x => Ok(x * 10),
                    })
                    .collect(),
            )
        })
        .with_batch("broken", |_: Vec<i32>| {
            Err::<Vec<Result<i32, String>>, _>("the model crashed".to_string())
        })
        .with_continuous("decode", move || Decoder {
            log: decoded.clone(),
            active: HashMap::new(),
        })
}

fn policy(size: usize, max_wait: Option<u64>) -> BatchPolicy {
    BatchPolicy {
        size,
        max_wait: max_wait.map(Duration::from_millis),
        slots: 1,
    }
}

async fn run_on(cluster: &LocalCluster, workflow: Workflow) -> RunOutcome {
    tokio::time::timeout(Duration::from_secs(20), cluster.run(workflow))
        .await
        .expect("the run finished")
        .unwrap()
}

#[workflow]
fn embed_all() -> i32 {
    let n = 40;
    let xs = items(n);
    let ys = embed(xs);
    sum(ys)
}

#[tokio::test(flavor = "multi_thread")]
async fn items_from_many_branches_share_batches() {
    let log = Arc::new(Log::default());
    let tasks = tasks(log.clone()).batch("embed", policy(8, None));
    let cluster = LocalCluster::uniform(1, Arc::new(tasks));
    let outcome = run_on(&cluster, embed_all().to_serializable("e".into())).await;
    // (0 + .. + 39) * 10, without the failed 13
    assert_eq!(outcome.results, [json!(7800 - 130)]);
    assert_eq!(outcome.failures, ["embed: unlucky"]);
    let sizes: Vec<usize> = log.steps().iter().map(Vec::len).collect();
    assert!(sizes.iter().all(|&size| size <= 8), "{sizes:?}");
    assert!(
        sizes.iter().any(|&size| size > 1),
        "no batch formed: {sizes:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lone_item_runs_at_once_unless_told_to_wait() {
    #[workflow]
    fn wf() -> i32 {
        let x = 5;
        embed(x)
    }

    for (max_wait, waits) in [(None, false), (Some(300), true)] {
        let tasks = tasks(Arc::default()).batch("embed", policy(8, max_wait));
        let cluster = LocalCluster::uniform(1, Arc::new(tasks));
        let started = Instant::now();
        let outcome = run_on(&cluster, wf().to_serializable("one".into())).await;
        assert_eq!(outcome.results, [json!(50)]);
        let waited = started.elapsed() >= Duration::from_millis(250);
        assert_eq!(
            waited,
            waits,
            "max_wait {max_wait:?}: {:?}",
            started.elapsed()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_step_fails_every_item_in_it() {
    #[workflow]
    fn wf() -> i32 {
        let n = 6;
        let xs = items(n);
        let ys = broken(xs);
        sum(ys)
    }

    // One step takes all six items.
    let tasks = tasks(Arc::default()).batch("broken", policy(6, Some(200)));
    let cluster = LocalCluster::uniform(1, Arc::new(tasks));
    let outcome = run_on(&cluster, wf().to_serializable("b".into())).await;
    assert_eq!(outcome.results, [json!(0)]);
    assert_eq!(outcome.failures, vec!["broken: the model crashed"; 6]);
}

#[tokio::test(flavor = "multi_thread")]
async fn short_items_overtake_a_long_one_in_continuous_batching() {
    #[workflow]
    fn wf() -> i32 {
        let xs = lengths();
        let ys = decode(xs);
        sum(ys)
    }

    #[task]
    fn lengths() -> Result<Vec<i32>>;

    let log = Arc::new(Log::default());
    let tasks = tasks(log.clone())
        .with("lengths", |_: ()| {
            Ok::<_, String>(vec![20, 1, 2, 1, 3, 1, 2, 1])
        })
        .batch("decode", policy(4, None));
    let cluster = LocalCluster::uniform(1, Arc::new(tasks));
    let outcome = run_on(&cluster, wf().to_serializable("d".into())).await;
    assert_eq!(outcome.results, [json!(31)]);
    let finished = log.finished.lock().unwrap().clone();
    assert_eq!(
        finished.last(),
        Some(&20),
        "the long item held others back: {finished:?}"
    );
    assert!(*log.max_active.lock().unwrap() <= 4);
    // Items were admitted while the long one was still running.
    assert!(
        log.steps()
            .iter()
            .filter(|admitted| !admitted.is_empty())
            .count()
            > 1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_share_batches() {
    #[workflow]
    fn shifted() -> i32 {
        let n = 20;
        let xs = items(n);
        let ys = shift(xs);
        let zs = embed(ys);
        sum(zs)
    }

    #[task]
    fn shift(x: i32) -> Result<i32>;

    let log = Arc::new(Log::default());
    let tasks = Arc::new(
        tasks(log.clone())
            .with("shift", |x: i32| Ok::<_, String>(x + 1000))
            .batch("embed", policy(64, None)),
    );
    let cluster = Arc::new(LocalCluster::uniform(1, tasks));
    let (a, b) = tokio::join!(
        run_on(&cluster, embed_all().to_serializable("a".into())),
        run_on(&cluster, shifted().to_serializable("b".into())),
    );
    assert_eq!(a.results, [json!(7800 - 130)]);
    assert_eq!(b.results, [json!(201900)]);
    let mixed = log
        .steps()
        .iter()
        .any(|step| step.iter().any(|&x| x < 1000) && step.iter().any(|&x| x >= 1000));
    assert!(mixed, "no step mixed the two runs: {:?}", log.steps());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_survives_its_worker_dying() {
    let embeds: Arc<dyn Tasks> = Arc::new(tasks(Arc::default()).batch("embed", policy(4, None)));
    let cluster = Arc::new(LocalCluster::uniform(3, embeds));
    let run = tokio::spawn({
        let cluster = cluster.clone();
        async move { cluster.run(embed_all().to_serializable("e".into())).await }
    });
    tokio::time::sleep(Duration::from_millis(60)).await;
    cluster.kill(2).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("the run finished")
        .unwrap()
        .unwrap();
    assert_eq!(outcome.results, [json!(7800 - 130)]);
    assert_eq!(outcome.failures, ["embed: unlucky"]);
}
