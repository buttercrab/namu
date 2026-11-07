//! Expressions that take values apart and put them together, with no task: a struct's field, a
//! tuple's element, a list's element by index, a tuple, and `format!`. They run where a branch
//! stands, on values that may be in memory or MessagePack from another worker.

use std::sync::Arc;

use namu::prelude::*;
use namu_core::ir::BuiltinOp;
use namu_engine::{FnTasks, LocalCluster, RunOutcome, Tasks};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

#[derive(Clone, Debug, Serialize, Deserialize, namu::Data)]
pub struct Page {
    pub url: String,
    pub words: i64,
    pub links: Vec<String>,
    pub stats: Stats,
}

#[derive(Clone, Debug, Serialize, Deserialize, namu::Data)]
pub struct Stats {
    pub score: f64,
    pub span: (i32, i32),
}

#[task]
fn fetch(url: String) -> Result<Page>;

#[task]
fn pages(n: i64) -> Result<Vec<Page>>;

#[task]
fn shout(text: String) -> Result<String>;

fn page(url: &str, words: i64) -> Page {
    Page {
        url: url.to_string(),
        words,
        links: vec![format!("{url}/a"), format!("{url}/b")],
        stats: Stats {
            score: words as f64 / 4.0,
            span: (1, words as i32),
        },
    }
}

fn tasks() -> FnTasks {
    FnTasks::new()
        .with("fetch", |url: String| Ok::<_, String>(page(&url, 10)))
        .with("pages", |n: i64| {
            Ok::<_, String>(
                (1..=n)
                    .map(|i| page(&format!("p{i}"), i * 3))
                    .collect::<Vec<_>>(),
            )
        })
        .with("shout", |text: String| Ok::<_, String>(text.to_uppercase()))
}

/// A run's results as JSON, and its failures, both sorted.
type Seen = (Vec<Json>, Vec<String>);

/// Runs `workflow` on one worker and on three over TCP, where values cross as MessagePack; both
/// must agree.
async fn run(workflow: namu_core::ir::Workflow) -> Seen {
    let tasks: Arc<dyn Tasks> = Arc::new(tasks());
    let one = LocalCluster::uniform(1, tasks.clone());
    let tcp = LocalCluster::tcp(vec![tasks; 3]).await.unwrap();
    let seen = |outcome: RunOutcome| -> Seen {
        let mut results: Vec<Json> = outcome
            .results
            .iter()
            .map(|value| rmp_serde::from_slice(&value.encode().unwrap()).unwrap())
            .collect();
        results.sort_by_key(ToString::to_string);
        let mut failures = outcome.failures;
        failures.sort();
        (results, failures)
    };
    let alone = seen(one.run(workflow.clone()).await.unwrap());
    let spread = seen(tcp.run(workflow).await.unwrap());
    assert_eq!(alone, spread, "one worker and three disagree");
    alone
}

async fn results(workflow: namu_core::ir::Workflow) -> Vec<Json> {
    let (results, failures) = run(workflow).await;
    assert!(failures.is_empty(), "{failures:?}");
    results
}

#[tokio::test(flavor = "multi_thread")]
async fn fields_index_and_tuples_take_values_apart() {
    #[workflow]
    fn wf() -> (String, i64, String, i32, f64) {
        let page = fetch("https://example.com");
        let first = page.links[0];
        let span = page.stats.span;
        (
            shout(page.url),
            page.words * 2,
            first,
            span.1 - span.0,
            page.stats.score,
        )
    }

    let ir = wf().to_serializable("fields".into());
    let ops: Vec<&BuiltinOp> = ir
        .operations
        .iter()
        .flat_map(|op| &op.builtins)
        .map(|builtin| &builtin.op)
        .collect();
    assert!(ops.contains(&&BuiltinOp::Field { name: "url".into() }));
    assert!(ops.contains(&&BuiltinOp::Field { name: "1".into() }));
    assert!(ops.contains(&&BuiltinOp::Index));
    assert!(ops.contains(&&BuiltinOp::Tuple));

    assert_eq!(
        results(ir).await,
        [json!([
            "HTTPS://EXAMPLE.COM",
            20,
            "https://example.com/a",
            9,
            2.5
        ])]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn format_writes_scalars_as_display_does() {
    #[workflow]
    fn wf() -> String {
        let page = each(pages(2));
        let words = page.words;
        let ratio = page.stats.score;
        let url = page.url;
        format!("{url}: {} words ({}), {{ok}} {}", words, ratio, words > 5)
    }

    assert_eq!(
        results(wf().to_serializable("format".into())).await,
        [
            json!("p1: 3 words (0.75), {ok} false"),
            json!("p2: 6 words (1.5), {ok} true"),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_index_out_of_bounds_fails_its_branch() {
    #[workflow]
    fn wf() -> String {
        let page = each(pages(3));
        page.links[2]
    }

    let (results, failures) = run(wf().to_serializable("bounds".into())).await;
    assert!(results.is_empty());
    assert_eq!(failures.len(), 3, "{failures:?}");
    assert!(failures[0].contains("out of bounds"), "{failures:?}");
}
