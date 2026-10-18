//! The smallest NAMU program: a workflow, its tasks as Rust functions, and a cluster of workers
//! in this process.
//!
//! `cargo run -p namu-basic`

use std::sync::Arc;

use namu::prelude::*;
use namu_engine::{FnTasks, LocalCluster};

// Tasks are declared by signature; the workflow is type-checked against these.
#[task]
fn add(a: i32, b: i32) -> Result<i32>;

#[task]
fn is_less(a: i32, b: i32) -> Result<bool>;

#[task]
fn range(from: i32, to: i32) -> Result<Vec<i32>>;

#[task]
fn square(x: i32) -> Result<i32>;

#[task]
fn sum(xs: Vec<i32>) -> Result<i32>;

/// The first Fibonacci number of at least 100.
#[workflow]
fn fibonacci() -> i32 {
    let mut a = 0;
    let mut b = 1;
    while is_less(b, 100) {
        let c = add(a, b);
        a = b;
        b = c;
    }
    b
}

/// 1² + 2² + ... + 10²: `square` takes an `i32` but gets a list, so it runs once per item, in
/// parallel; `sum` takes a `Vec<i32>` but gets per-item values, so it collects them.
#[workflow]
fn sum_of_squares() -> i32 {
    let xs = range(1, 11);
    let squares = square(xs);
    sum(squares)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let tasks = FnTasks::new()
        .with("add", |(a, b): (i32, i32)| Ok::<_, String>(a + b))
        .with("is_less", |(a, b): (i32, i32)| Ok::<_, String>(a < b))
        .with("range", |(from, to): (i32, i32)| {
            Ok::<_, String>((from..to).collect::<Vec<_>>())
        })
        .with("square", |x: i32| Ok::<_, String>(x * x))
        .with("sum", |xs: Vec<i32>| {
            Ok::<_, String>(xs.iter().sum::<i32>())
        });
    let cluster = LocalCluster::uniform(3, Arc::new(tasks));

    for (name, workflow) in [
        ("fibonacci", fibonacci().to_serializable("fibonacci".into())),
        (
            "sum_of_squares",
            sum_of_squares().to_serializable("sum_of_squares".into()),
        ),
    ] {
        let outcome = cluster.run(workflow).await?;
        let results: Vec<String> = outcome.results.iter().map(ToString::to_string).collect();
        println!("{name}: {}", results.join(", "));
    }
    Ok(())
}
