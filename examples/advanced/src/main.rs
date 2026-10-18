//! A data-labeling pipeline: records stream in, each is cleaned and labeled by a model that serves
//! many records at once (continuous batching), and the confident labels are reported.
//!
//! `cargo run -p namu-advanced`
//!
//! Everything runs in this process on a `LocalCluster`; the same workflow runs unchanged on
//! workers across machines, with the tasks built as WASM components or native libraries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use namu::prelude::*;
use namu_engine::tasks::{BatchPolicy, Continuous};
use namu_engine::{FnTasks, LocalCluster};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    text: String,
    label: String,
    confidence: f32,
}

/// Records as they arrive, one every few milliseconds.
#[task(stream)]
fn records(n: i32) -> Result<impl Iterator<Item = Result<String>>>;

#[task]
fn clean(record: String) -> Result<String>;

/// The model. Called per record; the runtime batches records from every branch.
#[task(batch)]
fn label(texts: Vec<String>) -> Vec<Result<Label>>;

/// Summarizes the confident labels.
#[task]
fn report(labels: Vec<Label>) -> Result<String>;

#[workflow]
fn labeling() -> String {
    let n = 200;
    let raw = records(n);
    let text = clean(raw);
    let labeled = label(text);
    report(labeled)
}

/// A stand-in for an LLM: a record takes one decoding step per word, and the model works on every
/// record in the session at once. A record leaves as soon as it is done.
struct Model {
    active: HashMap<u64, (String, usize)>,
    steps: Arc<Mutex<Vec<usize>>>,
}

impl Continuous for Model {
    type In = String;
    type Out = Label;

    fn step(&mut self, admit: Vec<(u64, String)>) -> Vec<(u64, Result<Label, String>)> {
        for (id, text) in admit {
            let words = text.split_whitespace().count();
            self.active.insert(id, (text, words));
        }
        self.steps.lock().unwrap().push(self.active.len());
        std::thread::sleep(Duration::from_millis(2));
        let mut done = Vec::new();
        self.active.retain(|&id, (text, left)| {
            *left -= 1;
            if *left > 0 {
                return true;
            }
            let label = if text.contains("refund") {
                "billing"
            } else {
                "general"
            };
            let confidence = 0.5 + (text.len() % 50) as f32 / 100.0;
            let text = text.clone();
            done.push((
                id,
                Ok(Label {
                    text,
                    label: label.into(),
                    confidence,
                }),
            ));
            false
        });
        done
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let steps = Arc::new(Mutex::new(Vec::new()));
    let model_steps = steps.clone();
    let tasks = FnTasks::new()
        .with_stream("records", |n: i32| {
            Ok::<_, String>((0..n).map(|i| {
                std::thread::sleep(Duration::from_millis(1));
                let topic = if i % 3 == 0 {
                    "a refund for order"
                } else {
                    "a question about"
                };
                Ok::<_, String>(format!(
                    "  Record {i}: {topic} {}  ",
                    "item ".repeat(i as usize % 6)
                ))
            }))
        })
        .with("clean", |record: String| {
            Ok::<_, String>(record.trim().to_lowercase())
        })
        .with_continuous("label", move || Model {
            active: HashMap::new(),
            steps: model_steps.clone(),
        })
        .batch(
            "label",
            BatchPolicy {
                size: 32,
                ..BatchPolicy::default()
            },
        )
        .with("report", |labels: Vec<Label>| {
            let total = labels.len();
            let confident: Vec<&Label> = labels.iter().filter(|l| l.confidence >= 0.6).collect();
            let billing = confident.iter().filter(|l| l.label == "billing").count();
            Ok::<_, String>(format!(
                "{total} records labeled, {} confidently, {billing} of those about billing",
                confident.len()
            ))
        });
    let cluster = LocalCluster::uniform(2, Arc::new(tasks));

    let started = Instant::now();
    let outcome = cluster
        .run(labeling().to_serializable("labeling".into()))
        .await?;
    let summary = outcome.results[0].as_str().unwrap_or_default();
    println!("{summary} in {:?}", started.elapsed());

    let steps = steps.lock().unwrap();
    let busiest = steps.iter().max().copied().unwrap_or(0);
    let average = steps.iter().sum::<usize>() as f32 / steps.len().max(1) as f32;
    println!(
        "model: {} steps, {average:.1} records per step on average, {busiest} at most",
        steps.len()
    );
    Ok(())
}
