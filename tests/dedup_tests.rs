//! Level-2 dedup (design §6.1): what every branch of a scope computes alike is computed once, by
//! the scope, as if it had been written before the fan-out.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use namu::prelude::*;
use namu_core::inputs::Inputs;
use namu_core::ir::Workflow;
use namu_engine::{FnTasks, LocalCluster, Tasks};
use serde_json::json;

#[task]
fn items(n: i32) -> Result<Vec<i32>>;

#[task]
fn load(size: i32) -> Result<i32>;

#[task]
fn classify(model: i32, item: i32) -> Result<i32>;

#[task]
fn is_even(item: i32) -> Result<bool>;

#[task]
fn sum(xs: Vec<i32>) -> Result<i32>;

/// How many times each task was called.
#[derive(Clone, Default)]
struct Calls(Arc<Mutex<HashMap<&'static str, usize>>>);

impl Calls {
    fn count(&self, task: &'static str) {
        *self.0.lock().unwrap().entry(task).or_default() += 1;
    }

    fn of(&self, task: &str) -> usize {
        self.0.lock().unwrap().get(task).copied().unwrap_or(0)
    }
}

fn tasks(calls: &Calls) -> FnTasks {
    let counted = |task: &'static str| {
        let calls = calls.clone();
        move || calls.count(task)
    };
    let (load, classify) = (counted("load"), counted("classify"));
    FnTasks::new()
        .with("items", |n: i32| {
            Ok::<_, String>((1..=n).collect::<Vec<_>>())
        })
        .with("load", move |size: i32| {
            load();
            Ok::<_, String>(size * 10)
        })
        .with("classify", move |(model, item): (i32, i32)| {
            classify();
            Ok::<_, String>(model + item)
        })
        .with("is_even", |item: i32| Ok::<_, String>(item % 2 == 0))
        .with("sum", |xs: Vec<i32>| {
            Ok::<_, String>(xs.iter().sum::<i32>())
        })
}

/// Runs `workflow` on three workers, and its one result.
async fn run(workflow: Workflow, inputs: serde_json::Value, tasks: FnTasks) -> serde_json::Value {
    let serde_json::Value::Object(inputs) = inputs else {
        panic!("inputs are an object")
    };
    let tasks: Arc<dyn Tasks> = Arc::new(tasks);
    let outcome = LocalCluster::uniform(3, tasks)
        .run_with(workflow, Inputs::from_json(inputs))
        .await
        .unwrap();
    let [result] = &outcome.results[..] else {
        panic!("one result: {:?}", outcome.results.len())
    };
    rmp_serde::from_slice(&result.encode().unwrap()).unwrap()
}

#[workflow]
fn shared(n: i32) -> i32 {
    let item = each(items(n)); // one branch per item
    let size = n + 1; // from before the fan-out: every branch alike
    let model = load(size);
    sum(classify(model, item))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_every_branch_makes_alike_is_made_once() {
    let calls = Calls::default();
    let workflow = shared().to_serializable("shared".into());
    let result = run(workflow, json!({ "n": 4 }), tasks(&calls)).await;
    // The model is 50, the items 1 to 4.
    assert_eq!(result, json!(4 * 50 + 10));
    assert_eq!((calls.of("load"), calls.of("classify")), (1, 4));
}

#[workflow]
fn nested() -> i32 {
    let outer = each(items(3));
    let inner = each(items(outer));
    let everywhere = load(7); // the root's: made once
    let per_outer = load(outer); // the outer item's: made once per outer item
    sum(sum(classify(everywhere + per_outer, inner)))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_value_is_made_at_the_depth_of_its_deepest_input() {
    let calls = Calls::default();
    let result = run(
        nested().to_serializable("nested".into()),
        json!({}),
        tasks(&calls),
    )
    .await;
    // Outer 1..=3, inner 1..=outer: 6 inner items in all.
    let expected: i32 = (1..=3)
        .map(|outer| {
            (1..=outer)
                .map(|inner| 70 + outer * 10 + inner)
                .sum::<i32>()
        })
        .sum();
    assert_eq!(result, json!(expected));
    assert_eq!((calls.of("load"), calls.of("classify")), (1 + 3, 6));
}

#[workflow]
fn conditional() -> i32 {
    let item = each(items(4));
    let mut label = item;
    if is_even(item) {
        label = classify(load(1), item);
    }
    sum(label)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_only_some_branches_make_is_made_by_each() {
    let calls = Calls::default();
    let result = run(
        conditional().to_serializable("conditional".into()),
        json!({}),
        tasks(&calls),
    )
    .await;
    assert_eq!(result, json!(1 + (10 + 2) + 3 + (10 + 4)));
    assert_eq!(calls.of("load"), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_with_side_effects_is_called_by_every_branch() {
    let calls = Calls::default();
    let workflow = shared().to_serializable("shared".into());
    let tasks = tasks(&calls).checkpoint("load");
    run(workflow, json!({ "n": 4 }), tasks).await;
    assert_eq!(calls.of("load"), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shared_call_is_made_even_for_no_items() {
    // As if written before the fan-out.
    let calls = Calls::default();
    let workflow = shared().to_serializable("shared".into());
    let result = run(workflow, json!({ "n": 0 }), tasks(&calls)).await;
    assert_eq!(result, json!(0));
    assert_eq!((calls.of("load"), calls.of("classify")), (1, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shared_call_is_not_made_if_the_list_cannot_be() {
    // Moved after the call that makes the list, not before it: that fails first.
    let calls = Calls::default();
    let tasks = tasks(&calls).with("items", |_: i32| Err::<Vec<i32>, _>("no items"));
    let tasks: Arc<dyn Tasks> = Arc::new(tasks);
    let inputs = Inputs::from_json(json!({ "n": 4 }).as_object().unwrap().clone());
    let outcome = LocalCluster::uniform(3, tasks)
        .run_with(shared().to_serializable("shared".into()), inputs)
        .await
        .unwrap();
    assert!(outcome.results.is_empty());
    assert_eq!(calls.of("load"), 0);
}
