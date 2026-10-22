//! Workflows run end to end on the engine. Every run is checked on one worker and on three.

mod common;

use std::sync::Arc;

use namu::prelude::each;
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
    assert_eq!(json(&outcome.results), [json!(728)]);
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
            json(&outcome.results),
            [json!((1..=20).map(|x| 50 * x + 10).sum::<i32>() - 20)]
        );
        assert_eq!(outcome.failures.len(), 1);
        // Every worker reported what it did: range, 20 splits, 100 maybe_fails, 20 + 1 sums.
        let calls: u64 = outcome.workers.values().map(|w| w.calls).sum();
        assert_eq!(calls, 142);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_context_outlives_a_long_call_while_others_churn() {
    // Each loop header's context is a node made for the call that follows. The second loop's
    // calls are slow, and while one branch waits in one, the other is still creating thousands of
    // nodes in its first loop. The waiting context must still be there when its call returns.
    #[workflow]
    fn wf() -> i32 {
        let xs = range(1, 3);
        let n = multiply_by_3_and_add_1(multiply_by_3_and_add_1(multiply_by_3_and_add_1(
            multiply_by_3_and_add_1(xs),
        )));
        let mut i = 0;
        while less_than(i, n) {
            i = add(i, 1);
        }
        let mut j = 0;
        while slowly_less_than(j, 2) {
            j = add(j, i);
        }
        j
    }

    let results = results(wf().to_serializable("churn".into())).await;
    assert_eq!(ints(&results), [850, 1660]);
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
    assert_eq!(json(&outcome.results), [json!(12502500)]);
    // 15,000 calls, each creating a node: a context owns only the chain of nodes still holding
    // live values, so a handful are held at once.
    let peak = outcome.workers[&1].peak_nodes;
    assert!(peak <= 64, "{peak} nodes held at once");
}

#[tokio::test(flavor = "multi_thread")]
async fn return_in_a_branch_drops_its_item() {
    #[workflow]
    fn wf() -> i32 {
        let xs = range(1, 5);
        let x = identity(xs);
        if less_than(x, 25) {
            return;
        }
        let y = double(x);
        sum(y)
    }

    // 10 and 20 return early, so only 30 and 40 reach the collect: (30 + 40) * 2.
    assert_eq!(
        results(wf().to_serializable("filter".into())).await,
        [json!(140)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn return_ends_the_workflow_early() {
    #[workflow]
    fn wf() -> i32 {
        let a = 1;
        if is_positive(a) {
            return double(a);
        }
        identity(a)
    }

    assert_eq!(
        results(wf().to_serializable("early".into())).await,
        [json!(2)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn independent_calls_in_a_branch_run_at_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[namu::task]
    fn probe(x: i32) -> anyhow::Result<i32>;

    #[workflow]
    fn wf() -> i32 {
        let a = 1;
        let b = probe(a);
        let c = probe(a);
        add(b, c)
    }

    // How many probes run at once, and the most at any moment.
    static RUNNING: AtomicUsize = AtomicUsize::new(0);
    static MOST: AtomicUsize = AtomicUsize::new(0);
    let tasks = tasks().with("probe", |x: i32| {
        let now = RUNNING.fetch_add(1, Ordering::SeqCst) + 1;
        MOST.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        RUNNING.fetch_sub(1, Ordering::SeqCst);
        Ok::<_, String>(x * 10)
    });
    let cluster = LocalCluster::uniform(1, Arc::new(tasks));
    let outcome = cluster
        .run(wf().to_serializable("parallel".into()))
        .await
        .unwrap();
    assert_eq!(json(&outcome.results), [json!(20)]);
    assert_eq!(
        MOST.load(Ordering::SeqCst),
        2,
        "the two probes ran one after the other"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_log_keeps_the_live_frontier_not_the_history() {
    #[workflow]
    fn wf() -> i32 {
        let mut i = 0;
        let mut total = 0;
        while less_than(i, 200) {
            let xs = range(1, 4);
            let ys = double(xs);
            total = add(total, sum(ys));
            i = add(i, 1);
        }
        total
    }

    let cluster = LocalCluster::uniform(1, Arc::new(tasks()));
    let outcome = cluster
        .run(wf().to_serializable("frontier".into()))
        .await
        .unwrap();
    // (10 + 20 + 30) * 2 per iteration.
    assert_eq!(json(&outcome.results), [json!(120 * 200)]);
    // 200 fan-outs log over a thousand nodes; the log drops those nothing live reaches.
    let peak = cluster.log().peak_nodes();
    assert!(peak <= 128, "the log held {peak} nodes at once");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attempted_failure_is_a_value() {
    #[workflow]
    fn wf() -> i32 {
        let x = range(1, 4);
        match attempt(maybe_fail(x)) {
            Ok(v) => v,
            Err(e) => length(e),
        }
    }

    // 20 fails with "intentional failure", 19 bytes; nothing is reported as a failure.
    assert_eq!(
        ints(&results(wf().to_serializable("attempt".into())).await),
        [10, 19, 30]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attempt_falls_back_and_merges_variables() {
    #[workflow]
    fn wf() -> i32 {
        let x = identity(range(1, 4));
        let mut fell_back = 0;
        let y = match attempt(maybe_fail(x)) {
            Ok(v) => v,
            Err(_) => {
                fell_back = 1;
                double(x)
            }
        };
        sum(add(y, fell_back))
    }

    // 10 + (40 + 1) + 30
    assert_eq!(
        results(wf().to_serializable("fallback".into())).await,
        [json!(81)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_call_is_retried_in_a_loop() {
    #[workflow]
    fn wf() -> i32 {
        let mut x = 20;
        let mut tries = 0;
        let mut out = 0;
        while less_than(tries, 3) {
            match attempt(maybe_fail(x)) {
                Ok(v) => {
                    out = v;
                    tries = 3;
                }
                Err(_) => {
                    x = add(x, 1);
                    tries = add(tries, 1);
                }
            }
        }
        out
    }

    // 20 fails; the retry with 21 succeeds.
    assert_eq!(
        results(wf().to_serializable("retry".into())).await,
        [json!(21)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn every_call_in_a_loop_has_its_own_key() {
    use std::collections::HashSet;
    use std::sync::Mutex;

    #[workflow]
    fn wf() -> i32 {
        let mut i = 0;
        while less_than(i, 50) {
            i = add(i, 1);
        }
        i
    }

    static KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let tasks = tasks().with("add", |(a, b): (i32, i32)| {
        let key = namu_engine::call_key().expect("a call has a key");
        KEYS.lock().unwrap().push(key.to_string());
        Ok::<_, String>(a + b)
    });
    let cluster = LocalCluster::uniform(1, Arc::new(tasks));
    let outcome = cluster
        .run(wf().to_serializable("keys".into()))
        .await
        .unwrap();
    assert_eq!(json(&outcome.results), [json!(50)]);
    let keys = KEYS.lock().unwrap();
    assert_eq!(keys.len(), 50);
    assert_eq!(keys.iter().collect::<HashSet<_>>().len(), 50);
}

#[tokio::test(flavor = "multi_thread")]
async fn break_leaves_the_loop_with_the_values_it_had() {
    #[workflow]
    fn wf() -> i32 {
        let mut i = 0;
        let mut total = 0;
        while less_than(i, 100) {
            total = add(total, i);
            if less_than(30, total) {
                break;
            }
            i = add(i, 1);
        }
        // 0 + 1 + .. + 8 = 36 is the first total over 30, at i = 8.
        add(add(total, total), i)
    }

    assert_eq!(
        results(wf().to_serializable("break".into())).await,
        [json!(80)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_skips_the_rest_of_the_body() {
    #[workflow]
    fn wf() -> i32 {
        let mut i = 0;
        let mut odd = 0;
        while less_than(i, 10) {
            i = add(i, 1);
            if is_even(i) {
                continue;
            }
            odd = add(odd, i);
        }
        // 1 + 3 + 5 + 7 + 9
        odd
    }

    assert_eq!(
        results(wf().to_serializable("continue".into())).await,
        [json!(25)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn break_and_continue_act_on_the_innermost_loop() {
    #[workflow]
    fn wf() -> i32 {
        let mut i = 0;
        let mut pairs = 0;
        while less_than(i, 4) {
            i = add(i, 1);
            if is_even(i) {
                continue;
            }
            let mut j = 0;
            while less_than(j, 100) {
                j = add(j, 1);
                if less_than(i, j) {
                    break;
                }
                pairs = add(pairs, 1);
            }
        }
        // i = 1 counts j = 1; i = 3 counts j = 1, 2, 3.
        pairs
    }

    assert_eq!(
        results(wf().to_serializable("nested".into())).await,
        [json!(4)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loop_in_a_fanned_out_branch_can_break() {
    #[workflow]
    fn wf() -> i32 {
        let x = identity(range(1, 4));
        let mut n = 0;
        while less_than(n, 1000) {
            if less_than(x, n) {
                break;
            }
            n = add(n, 7);
        }
        // The first multiple of 7 over 10, 20, 30: 14, 21, 35.
        sum(n)
    }

    assert_eq!(
        results(wf().to_serializable("fan_break".into())).await,
        [json!(70)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_overlaps_a_chain_it_does_not_read_from() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[namu::task]
    fn step(x: i32) -> anyhow::Result<i32>;

    #[workflow]
    fn wf() -> i32 {
        let x = 1;
        let a = step(x);
        let b = step(a);
        let c = step(x);
        add(b, c)
    }

    static RUNNING: AtomicUsize = AtomicUsize::new(0);
    static MOST: AtomicUsize = AtomicUsize::new(0);
    let tasks = tasks().with("step", |x: i32| {
        let now = RUNNING.fetch_add(1, Ordering::SeqCst) + 1;
        MOST.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        RUNNING.fetch_sub(1, Ordering::SeqCst);
        Ok::<_, String>(x + 1)
    });
    let cluster = LocalCluster::uniform(1, Arc::new(tasks));
    let outcome = cluster
        .run(wf().to_serializable("dataflow".into()))
        .await
        .unwrap();
    // b = 3, c = 2
    assert_eq!(json(&outcome.results), [json!(5)]);
    assert_eq!(MOST.load(Ordering::SeqCst), 2, "c waited for the chain");
}

#[tokio::test(flavor = "multi_thread")]
async fn in_a_group_the_first_failure_in_order_is_the_one_reported() {
    use std::time::Duration;

    #[namu::task]
    fn fail_slowly(x: i32) -> anyhow::Result<i32>;

    #[namu::task]
    fn fail_fast(x: i32) -> anyhow::Result<i32>;

    #[workflow]
    fn wf() -> i32 {
        let x = 1;
        let a = fail_slowly(x);
        let b = fail_fast(x);
        add(a, b)
    }

    let tasks = tasks()
        .with("fail_slowly", |_: i32| {
            std::thread::sleep(Duration::from_millis(100));
            Err::<i32, _>("first".to_string())
        })
        .with("fail_fast", |_: i32| Err::<i32, _>("second".to_string()));
    let cluster = LocalCluster::uniform(1, Arc::new(tasks));
    let outcome = cluster
        .run(wf().to_serializable("failures".into()))
        .await
        .unwrap();
    assert_eq!(outcome.failures, ["fail_slowly: first"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loop_of_builtins_calls_no_task() {
    #[workflow]
    fn wf() -> i32 {
        let mut a = 0;
        let mut b = 1;
        while a < 100 {
            let c = a + b;
            a = b;
            b = c;
        }
        a * 2 - b % 7
    }

    // a = 144, b = 233: 288 - 2
    assert_eq!(
        results(wf().to_serializable("fibonacci_builtins".into())).await,
        [json!(286)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn builtins_mix_with_tasks_and_fan_out() {
    #[workflow]
    fn wf() -> i32 {
        let x = identity(range(1, 5));
        let mut y = x * 2;
        y += 1;
        let big = y > 50 && !(y == 61);
        let z = if big { -y } else { y };
        sum(z)
    }

    // x = 10, 20, 30, 40; y = 21, 41, 61, 81; big: 81 only; z = 21, 41, 61, -81
    assert_eq!(
        results(wf().to_serializable("mixed".into())).await,
        [json!(42)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retry_loop_reads_as_rust() {
    #[workflow]
    fn wf() -> i32 {
        let mut x = 20;
        let mut tries = 0;
        let mut out = 0;
        while tries < 3 {
            match attempt(maybe_fail(x)) {
                Ok(v) => {
                    out = v;
                    break;
                }
                Err(_) => {
                    x += 1;
                    tries += 1;
                }
            }
        }
        out + tries * 100
    }

    // 20 fails once; 21 goes through.
    assert_eq!(
        results(wf().to_serializable("retry_builtins".into())).await,
        [json!(121)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn strings_concatenate_and_compare() {
    #[workflow]
    fn wf() -> bool {
        let s = exclaim("na") + "mu";
        s == "na!mu"
    }

    assert_eq!(
        results(wf().to_serializable("strings".into())).await,
        [json!(true)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_overflow_fails_its_branch() {
    #[workflow]
    fn wf() -> i32 {
        let x = identity(range(1, 3));
        let big = 2147483630;
        x + big
    }

    // 10 fits; 20 overflows i32 and fails its branch only.
    let outcome = run(wf().to_serializable("overflow".into())).await;
    assert_eq!(ints(&outcome.results), [2147483640]);
    assert_eq!(outcome.failures.len(), 1);
    assert!(
        outcome.failures[0].contains("overflow adding"),
        "{:?}",
        outcome.failures
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reduce_folds_every_item_in_order() {
    #[workflow]
    fn wf() -> String {
        let x = identity(range(1, 11));
        let y = double(x);
        join("", y)
    }

    // In branch order however the branches end, on any number of workers.
    assert_eq!(
        results(wf().to_serializable("reduce".into())).await,
        [json!("20,40,60,80,100,120,140,160,180,200")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reduce_passes_over_failed_and_filtered_branches() {
    #[workflow]
    fn wf() -> String {
        let x = maybe_fail(range(1, 6));
        if x == 40 {
            return;
        }
        join("start", x)
    }

    // 20 failed and 40 returned early: neither is folded.
    let outcome = run(wf().to_serializable("reduce_skips".into())).await;
    assert_eq!(outcome.results, [json!("start,10,30,50")]);
    assert_eq!(outcome.failures, ["maybe_fail: intentional failure"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reduce_over_nothing_is_its_first_accumulator() {
    #[workflow]
    fn wf() -> String {
        let x = identity(range(1, 1));
        join("nothing", x)
    }

    assert_eq!(
        results(wf().to_serializable("reduce_empty".into())).await,
        [json!("nothing")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scope_that_reduces_opens_where_its_reduce_task_runs() {
    #[workflow]
    fn wf() -> String {
        let x = identity(range(1, 5));
        join("", x)
    }

    // Only worker 3 runs `join`: the branch moves there before it fans out, so worker 3 owns the
    // scope and folds it.
    let without_join: Arc<dyn Tasks> = Arc::new(
        FnTasks::new()
            .with("range", |(start, end): (i32, i32)| {
                Ok::<_, String>((start..end).map(|x| x * 10).collect::<Vec<_>>())
            })
            .with("identity", Ok::<i32, String>),
    );
    let all: Arc<dyn Tasks> = Arc::new(tasks());
    let cluster = LocalCluster::new(vec![without_join.clone(), without_join, all]);
    let outcome = cluster
        .run(wf().to_serializable("reduce_moves".into()))
        .await
        .unwrap();
    assert!(outcome.succeeded(), "{:?}", outcome.failures);
    assert_eq!(json(&outcome.results), [json!("10,20,30,40")]);
}

#[test]
fn a_reduce_must_start_from_a_value_made_before_its_fan_out() {
    #[workflow]
    fn wf() -> String {
        let x = identity(range(1, 4));
        let start = exclaim("per item");
        join(start, x)
    }

    let error = namu_core::scopes::collect_ops(&wf().to_serializable("bad".into())).unwrap_err();
    assert!(
        error.to_string().contains("make it before the fan-out"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn windows_continue_once_per_n_items_in_order() {
    #[workflow]
    fn wf() -> String {
        let x = each(range(1, 8));
        let batch = window(x, 3);
        let total = sum(batch);
        join("", total)
    }

    // Windows [10, 20, 30], [40, 50, 60], [70]: one sum each, folded in window order.
    assert_eq!(
        results(wf().to_serializable("windows".into())).await,
        [json!("60,150,70")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_window_counts_the_items_that_reach_it() {
    #[workflow]
    fn wf() -> String {
        let x = maybe_fail(range(1, 6));
        let batch = window(x, 2);
        let n = count(batch);
        let total = sum(batch);
        join("", n * 1000 + total)
    }

    // 20 failed: windows [10, 30], [40, 50].
    let outcome = run(wf().to_serializable("windows_skip".into())).await;
    assert_eq!(outcome.results, [json!("2040,2090")]);
    assert_eq!(outcome.failures, ["maybe_fail: intentional failure"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_items_make_no_windows() {
    #[workflow]
    fn wf() -> String {
        let x = each(range(1, 1));
        let batch = window(x, 3);
        let total = sum(batch);
        join("none", total)
    }

    assert_eq!(
        results(wf().to_serializable("windows_empty".into())).await,
        [json!("none")]
    );
}
