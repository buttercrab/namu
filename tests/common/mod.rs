//! Test tasks, and a runner that checks every workflow on one worker and on several.

#![allow(dead_code)]

use std::sync::Arc;

use namu::prelude::*;
use namu_core::ir::Workflow;
use namu_engine::{FnTasks, LocalCluster, RunOutcome};
use serde_json::Value;

#[task]
pub fn add(a: i32, b: i32) -> Result<i32>;

#[task]
pub fn exclaim(s: String) -> Result<String>;

#[task]
pub fn is_positive(v: i32) -> Result<bool>;

#[task]
pub fn double(v: i32) -> Result<i32>;

#[task]
pub fn identity(v: i32) -> Result<i32>;

#[task]
pub fn is_negative(v: i32) -> Result<bool>;

#[task]
pub fn less_than(a: i32, b: i32) -> Result<bool>;

#[task]
pub fn is_even(n: i32) -> Result<bool>;

#[task]
pub fn divide_by_2(n: i32) -> Result<i32>;

#[task]
pub fn multiply_by_3_and_add_1(n: i32) -> Result<i32>;

#[task]
pub fn not_one(n: i32) -> Result<bool>;

/// `[start * 10, (start + 1) * 10, ..]` up to `end`, exclusive.
#[task]
pub fn range(start: i32, end: i32) -> Result<Vec<i32>>;

/// `[n, n + 1, .., n + k - 1]`.
#[task]
pub fn split(n: i32, k: i32) -> Result<Vec<i32>>;

/// Fails for 20.
#[task]
pub fn maybe_fail(v: i32) -> Result<i32>;

#[task]
pub fn sum(xs: Vec<i32>) -> Result<i32>;

#[task]
pub fn count(xs: Vec<i32>) -> Result<i32>;

pub fn tasks() -> FnTasks {
    FnTasks::new()
        .with("add", |(a, b): (i32, i32)| ok(a + b))
        .with("exclaim", |s: String| ok(format!("{s}!")))
        .with("is_positive", |v: i32| ok(v > 0))
        .with("double", |v: i32| ok(v * 2))
        .with("identity", ok::<i32>)
        .with("is_negative", |v: i32| ok(v < 0))
        .with("less_than", |(a, b): (i32, i32)| ok(a < b))
        .with("is_even", |n: i32| ok(n % 2 == 0))
        .with("divide_by_2", |n: i32| ok(n / 2))
        .with("multiply_by_3_and_add_1", |n: i32| ok(n * 3 + 1))
        .with("not_one", |n: i32| ok(n != 1))
        .with("range", |(start, end): (i32, i32)| {
            ok((start..end).map(|x| x * 10).collect::<Vec<_>>())
        })
        .with("split", |(n, k): (i32, i32)| {
            ok((0..k).map(|x| n + x).collect::<Vec<_>>())
        })
        .with("maybe_fail", |v: i32| match v {
            20 => Err("intentional failure".to_string()),
            v => Ok(v),
        })
        .with("sum", |xs: Vec<i32>| ok(xs.iter().sum::<i32>()))
        .with("count", |xs: Vec<i32>| ok(xs.len() as i32))
}

fn ok<T>(value: T) -> Result<T, String> {
    Ok(value)
}

/// Runs `workflow` on a single worker and on three, checks that both give the same outcome, and
/// returns it with results sorted.
pub async fn run(workflow: Workflow) -> RunOutcome {
    let tasks = Arc::new(tasks());
    let one = LocalCluster::uniform(1, tasks.clone());
    let three = LocalCluster::uniform(3, tasks);
    let alone = sorted(one.run(workflow.clone()).await.expect("run on one worker"));
    let spread = sorted(three.run(workflow).await.expect("run on three workers"));
    assert_eq!(alone, spread, "one worker and three workers disagree");
    alone
}

/// The results of a run that must succeed.
pub async fn results(workflow: Workflow) -> Vec<Value> {
    let outcome = run(workflow).await;
    assert!(outcome.succeeded(), "failures: {:?}", outcome.failures);
    outcome.results
}

fn sorted(mut outcome: RunOutcome) -> RunOutcome {
    outcome.results.sort_by_key(ToString::to_string);
    outcome.failures.sort();
    outcome
}

/// Integer results, in ascending order.
pub fn ints(values: &[Value]) -> Vec<i64> {
    let mut ints: Vec<i64> = values
        .iter()
        .map(|v| v.as_i64().expect("an integer result"))
        .collect();
    ints.sort();
    ints
}
