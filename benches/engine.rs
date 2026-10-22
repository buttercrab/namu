//! Engine benchmarks: `cargo bench --bench engine`.
//!
//! - `loop`: a 5,000-iteration loop in one process; the cost of a step through the kernel, the
//!   value tree and an in-process task.
//! - `fan_out`: 10,000 items fanned out, each doubled, and collected.
//! - `value`: cloning, encoding and decoding a value.
//! - `find`: a lookup at the root of a 2,000-node chain, by jumping (binary lifting) and by walking
//!   parent by parent.

use std::sync::Arc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use namu::Value;
use namu::prelude::*;
use namu_engine::tree::Node;
use namu_engine::{FnTasks, LocalCluster};
use serde::{Deserialize, Serialize};

#[task]
fn add(a: i32, b: i32) -> Result<i32>;

#[task]
fn less_than(a: i32, b: i32) -> Result<bool>;

#[task]
fn range(n: i32) -> Result<Vec<i32>>;

#[task]
fn double(x: i32) -> Result<i32>;

#[task]
fn sum(xs: Vec<i32>) -> Result<i32>;

#[workflow]
fn counting() -> i32 {
    let mut i = 0;
    let mut total = 0;
    while less_than(i, 5000) {
        i = add(i, 1);
        total = add(total, i);
    }
    total
}

/// `counting`, with its arithmetic and comparison as built-ins: no task calls.
#[workflow]
fn counting_builtins() -> i32 {
    let mut i = 0;
    let mut total = 0;
    while i < 5000 {
        i += 1;
        total += i;
    }
    total
}

#[workflow]
fn doubling() -> i32 {
    let xs = range(10000);
    let ys = double(xs);
    sum(ys)
}

fn tasks() -> FnTasks {
    FnTasks::new()
        .with("add", |(a, b): (i32, i32)| {
            Ok::<_, String>(a.wrapping_add(b))
        })
        .with("less_than", |(a, b): (i32, i32)| Ok::<_, String>(a < b))
        .with("range", |n: i32| {
            Ok::<_, String>((0..n).collect::<Vec<_>>())
        })
        .with("double", |x: i32| Ok::<_, String>(x * 2))
        .with("sum", |xs: Vec<i32>| {
            Ok::<_, String>(xs.iter().sum::<i32>())
        })
}

fn engine(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    // Workers start on the runtime.
    let _entered = runtime.enter();
    let cluster = LocalCluster::uniform(1, Arc::new(tasks()));
    let counting = counting().to_serializable("counting".into());
    let counting_builtins = counting_builtins().to_serializable("counting_builtins".into());
    let doubling = doubling().to_serializable("doubling".into());
    c.bench_function("loop", |b| {
        b.iter(|| runtime.block_on(cluster.run(counting.clone())).unwrap())
    });
    c.bench_function("loop_builtins", |b| {
        b.iter(|| {
            runtime
                .block_on(cluster.run(counting_builtins.clone()))
                .unwrap()
        })
    });
    c.bench_function("fan_out", |b| {
        b.iter(|| runtime.block_on(cluster.run(doubling.clone())).unwrap())
    });
}

#[derive(Clone, Serialize, Deserialize, namu::Data)]
struct Doc {
    title: String,
    words: Vec<String>,
    score: f64,
}

fn value(c: &mut Criterion) {
    let doc = Doc {
        title: "a document".into(),
        words: (0..64).map(|i| format!("word{i}")).collect(),
        score: 0.5,
    };
    let value = Value::new(doc);
    let encoded = value.encode().unwrap();
    c.bench_function("value/clone", |b| b.iter(|| value.clone()));
    c.bench_function("value/encode", |b| b.iter(|| value.encode().unwrap()));
    c.bench_function("value/decode", |b| {
        b.iter_batched(
            || Value::encoded(encoded.clone()),
            |value| value.take::<Doc>().unwrap(),
            BatchSize::SmallInput,
        )
    });
}

fn find(c: &mut Criterion) {
    let mut chain = Arc::new(Node::root(0));
    for id in 1..2000 {
        chain = Arc::new(Node::new(
            id,
            chain,
            vec![(id as usize, Value::new(id as i64))],
        ));
    }
    c.bench_function("find/jump", |b| {
        b.iter(|| chain.find(1).unwrap().unwrap().clone())
    });
    c.bench_function("find/walk", |b| {
        b.iter(|| {
            let mut node: &Node = &chain;
            loop {
                if let Some(value) = node.get(1) {
                    break value.clone();
                }
                node = node.parent().unwrap();
            }
        })
    });
}

criterion_group!(benches, engine, value, find);
criterion_main!(benches);
