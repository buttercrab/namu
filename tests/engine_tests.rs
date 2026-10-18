//! Workflows run end to end on the engine. Every run is checked on one worker and on three.

mod common;

use std::sync::Arc;

use namu::workflow;
use namu_engine::{FnTasks, LocalCluster, Tasks};
use serde_json::json;

use crate::common::*;

#[tokio::test(flavor = "multi_thread")]
async fn engine_executes_simple_workflow() {
    #[workflow]
    fn wf() -> i32 {
        let a = 1;
        let b = 2;
        add(a, b)
    }

    assert_eq!(
        results(wf().to_serializable("simple".into())).await,
        [json!(3)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn engine_executes_fibonacci_workflow() {
    #[workflow]
    fn wf() -> i32 {
        let mut a = 0;
        let mut b = 1;
        while less_than(a, 10) {
            let c = add(a, b);
            a = b;
            b = c;
        }
        b
    }

    assert_eq!(
        results(wf().to_serializable("fibonacci".into())).await,
        [json!(21)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn loop_phis_are_parallel_assignments() {
    // `t = a; a = b; b = t;` swaps a and b. Phis applied one at a time would read the freshly
    // written `a` and produce a == b == 2.
    #[workflow]
    fn wf() -> i32 {
        let mut a = 1;
        let mut b = 2;
        let mut i = 0;
        while less_than(i, 1) {
            let t = a;
            a = b;
            b = t;
            i = add(i, 1);
        }
        add(add(a, a), b)
    }

    // a = 2, b = 1 -> 2 + 2 + 1
    assert_eq!(
        results(wf().to_serializable("swap".into())).await,
        [json!(5)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn string_literals_keep_escaped_characters() {
    #[workflow]
    fn wf() -> String {
        let s = "say \"hi\"\n";
        exclaim(s)
    }

    let results = results(wf().to_serializable("exclaim".into())).await;
    assert_eq!(results, [json!("say \"hi\"\n!")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn uncollected_branches_each_return_a_result() {
    // `split` takes an `i32` but gets a list: one branch per item, and again per item of each
    // split. Nothing collects, so every branch returns its own result.
    #[workflow]
    fn wf() -> i32 {
        let a = range(1, 4);
        let b = split(a, 3);
        identity(b)
    }

    let results = results(wf().to_serializable("list".into())).await;
    assert_eq!(ints(&results), [10, 11, 12, 20, 21, 22, 30, 31, 32]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_branch_is_dropped_and_reported() {
    #[workflow]
    fn wf() -> i32 {
        let a = range(1, 4);
        maybe_fail(a)
    }

    let outcome = run(wf().to_serializable("failure".into())).await;
    assert_eq!(ints(&outcome.results), [10, 30]);
    assert_eq!(outcome.failures, ["maybe_fail: intentional failure"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn fan_out_then_collect() {
    #[workflow]
    fn wf() -> i32 {
        let xs = range(1, 5);
        let ys = double(xs);
        sum(ys)
    }

    // (10 + 20 + 30 + 40) * 2
    assert_eq!(
        results(wf().to_serializable("collect".into())).await,
        [json!(200)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_scopes_collect_inside_out() {
    #[workflow]
    fn wf() -> i32 {
        let xs = range(1, 4);
        let parts = split(xs, 2);
        let per_x = sum(parts);
        sum(per_x)
    }

    // (10 + 11) + (20 + 21) + (30 + 31)
    assert_eq!(
        results(wf().to_serializable("nested".into())).await,
        [json!(123)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_branches_are_left_out_of_the_collect() {
    #[workflow]
    fn wf() -> i32 {
        let xs = range(1, 4);
        let ys = maybe_fail(xs);
        count(ys)
    }

    let outcome = run(wf().to_serializable("partial".into())).await;
    assert_eq!(outcome.results, [json!(2)]);
    assert_eq!(outcome.failures.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scope_whose_branches_all_fail_collects_nothing() {
    #[workflow]
    fn wf() -> i32 {
        let xs = range(2, 3);
        let ys = maybe_fail(xs);
        count(ys)
    }

    let outcome = run(wf().to_serializable("all_failed".into())).await;
    assert_eq!(outcome.results, [json!(0)]);
    assert_eq!(outcome.failures.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn fanning_out_over_an_empty_list_collects_nothing() {
    #[workflow]
    fn wf() -> i32 {
        let xs = range(5, 5);
        let ys = double(xs);
        count(ys)
    }

    assert_eq!(
        results(wf().to_serializable("empty".into())).await,
        [json!(0)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn constants_are_visible_after_a_collect() {
    // `k` is written between the fan-out and the collect, but it is a constant, so it lives in
    // the root context and survives the end of the scope.
    #[workflow]
    fn wf() -> i32 {
        let xs = range(1, 3);
        let ys = double(xs);
        let k = 7;
        let total = sum(ys);
        add(total, k)
    }

    assert_eq!(
        results(wf().to_serializable("constant".into())).await,
        [json!(67)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn branches_move_to_the_worker_that_serves_their_task() {
    #[workflow]
    fn wf() -> i32 {
        let xs = range(1, 9);
        let ys = double(xs);
        let zs = add(ys, 1);
        sum(zs)
    }

    // Only worker 2 serves `double`, and only worker 1 serves the rest, so every branch moves
    // to worker 2 and back.
    let rest = FnTasks::new()
        .with("range", |(start, end): (i32, i32)| {
            Ok::<_, String>((start..end).map(|x| x * 10).collect::<Vec<_>>())
        })
        .with("add", |(a, b): (i32, i32)| Ok::<_, String>(a + b))
        .with("sum", |xs: Vec<i32>| {
            Ok::<_, String>(xs.iter().sum::<i32>())
        });
    let double = FnTasks::new().with("double", |v: i32| Ok::<_, String>(v * 2));
    let cluster = LocalCluster::new(vec![Arc::new(rest), Arc::new(double)]);
    let outcome = cluster
        .run(wf().to_serializable("handoff".into()))
        .await
        .unwrap();
    // (10 + .. + 80) * 2 + 8
    assert_eq!(outcome.results, [json!(728)]);
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
    // Worker 1 ran range, the 8 adds and sum; worker 2 the 8 doubles.
    assert_eq!(outcome.workers[&1].calls, 10);
    assert_eq!(outcome.workers[&2].calls, 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_runs_end_exactly_once_with_the_same_result() {
    #[workflow]
    fn wf() -> i32 {
        let xs = range(1, 21);
        let parts = split(xs, 5);
        let ys = maybe_fail(parts);
        let per_x = sum(ys);
        sum(per_x)
    }

    let tasks: Arc<dyn Tasks> = Arc::new(tasks());
    let local = Arc::new(LocalCluster::uniform(4, tasks.clone()));
    let tcp = Arc::new(LocalCluster::tcp(vec![tasks; 4]).await.unwrap());
    let runs: Vec<_> = (0..64)
        .map(|i| {
            let cluster = if i % 2 == 0 {
                local.clone()
            } else {
                tcp.clone()
            };
            tokio::spawn(async move { cluster.run(wf().to_serializable("stress".into())).await })
        })
        .collect();
    for run in runs {
        let outcome = run.await.unwrap().unwrap();
        // x = 1..=20 is item 10x, split into 10x..10x+4: each contributes 50x + 10. The part 20
        // (of x = 2) fails and is left out.
        assert_eq!(
            outcome.results,
            [json!((1..=20).map(|x| 50 * x + 10).sum::<i32>() - 20)]
        );
        assert_eq!(outcome.failures.len(), 1);
        // Every worker reported what it did: range, 20 splits, 100 maybe_fails, 20 + 1 sums.
        let calls: u64 = outcome.workers.values().map(|w| w.calls).sum();
        assert_eq!(calls, 142);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_loop_holds_a_flat_number_of_nodes() {
    #[workflow]
    fn wf() -> i32 {
        let mut i = 0;
        let mut total = 0;
        while less_than(i, 5000) {
            i = add(i, 1);
            total = add(total, i);
        }
        total
    }

    let cluster = LocalCluster::uniform(1, Arc::new(tasks()));
    let outcome = cluster
        .run(wf().to_serializable("loop".into()))
        .await
        .unwrap();
    // 1 + 2 + .. + 5000, overflowing nothing in i32.
    assert_eq!(outcome.results, [json!(12502500)]);
    // 15,000 calls, each creating a node: without placement and sweeping, all would be held.
    let peak = outcome.workers[&1].peak_nodes;
    assert!(peak < 2500, "{peak} nodes held at once");
}
