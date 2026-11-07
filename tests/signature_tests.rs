//! A workflow is checked against what its tasks take and give before it runs: in process here,
//! as the orchestrator does when a workflow is published. Types are compared by the shapes that
//! cross, as serde writes them, not by their names.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use namu::prelude::*;
use namu_core::types::{Def, Defs, Describe, Type};
use namu_engine::{FnTasks, LocalCluster, Tasks};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Page {
    pub url: String,
    pub words: i64,
}

/// The task's own page, written apart from the workflow's: it needs `url`, can do without
/// `lang`, and has no use for `words`.
#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Doc {
    pub url: String,
    pub lang: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Titled {
    pub title: String,
}

#[task]
fn fetch(url: String) -> Result<Page>;

#[task]
fn language(page: Page) -> Result<String>;

#[workflow]
fn wf() -> String {
    let page = fetch("https://example.com");
    language(page)
}

/// Runs `wf` on one worker with `tasks`, and how many calls ran.
async fn run(tasks: FnTasks) -> (anyhow::Result<Vec<serde_json::Value>>, usize) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let tasks = tasks.with("fetch", move |url: String| {
        counted.fetch_add(1, Ordering::SeqCst);
        Ok::<_, String>(Page { url, words: 3 })
    });
    let tasks: Arc<dyn Tasks> = Arc::new(tasks);
    let outcome = LocalCluster::uniform(1, tasks)
        .run(wf().to_serializable("wf".into()))
        .await
        .map(|outcome| {
            let results = outcome.results.iter();
            results
                .map(|value| rmp_serde::from_slice(&value.encode().unwrap()).unwrap())
                .collect()
        });
    (outcome, calls.load(Ordering::SeqCst))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_with_its_own_types_runs_when_their_shapes_fit() {
    let tasks = FnTasks::new().with("language", |doc: Doc| {
        Ok::<_, String>(format!("{}: {}", doc.url, doc.lang.unwrap_or("en".into())))
    });
    let (results, _) = run(tasks).await;
    assert_eq!(results.unwrap(), [json!("https://example.com: en")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_that_does_not_fit_is_refused_before_anything_runs() {
    let tasks = FnTasks::new().with("language", |titled: Titled| Ok::<_, String>(titled.title));
    let (results, calls) = run(tasks).await;
    let err = format!("{:#}", results.unwrap_err());
    assert!(
        err.ends_with(
            "call to `language` (op 1): argument 1: `Page` has no field `title`, which `Titled` \
             needs"
        ),
        "{err}"
    );
    assert_eq!(calls, 0);

    let tasks = FnTasks::new().with("language", |_: Page| Ok::<_, String>(3));
    let err = format!("{:#}", run(tasks).await.0.unwrap_err());
    assert!(err.ends_with("result: `i32` is read as `String`"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_must_fit_every_worker_that_serves_its_task() {
    // Two workers serve `language`, and only one's takes a page: a call may go to either.
    let fetch = |url: String| Ok::<_, String>(Page { url, words: 3 });
    let fits = FnTasks::new()
        .with("fetch", fetch)
        .with("language", |doc: Doc| Ok::<_, String>(doc.url));
    let other = FnTasks::new()
        .with("fetch", fetch)
        .with("language", |titled: Titled| Ok::<_, String>(titled.title));
    let tasks: Vec<Arc<dyn Tasks>> = vec![Arc::new(fits), Arc::new(other)];
    let err = LocalCluster::new(tasks)
        .run(wf().to_serializable("wf".into()))
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("`Page` has no field `title`"),
        "{err:#}"
    );
}

#[derive(Serialize, Deserialize, namu::Describe)]
#[serde(rename_all = "camelCase")]
pub struct Camel {
    pub page_url: String,
    #[serde(rename = "n")]
    pub count: i64,
    #[serde(skip)]
    pub cache: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

#[derive(Serialize, Deserialize, namu::Describe)]
#[serde(rename_all = "snake_case")]
pub enum Fetched {
    NotFound,
    #[serde(rename = "ok")]
    Found(String),
}

#[derive(Serialize, Deserialize, namu::Describe)]
pub struct Flat {
    #[serde(flatten)]
    pub page: Camel,
}

#[derive(Serialize, Deserialize, namu::Describe)]
pub struct Defaulted {
    #[serde(default)]
    pub words: i64,
}

fn def<T: Describe>() -> Option<Def> {
    let mut defs = Defs::new();
    let Type::Named(name) = T::describe(&mut defs) else {
        panic!("named")
    };
    defs.remove(&name)
}

#[test]
fn fields_and_variants_are_named_as_serde_names_them() {
    let int = Type::Int {
        bits: 64,
        signed: true,
    };
    assert_eq!(
        def::<Camel>(),
        Some(Def::Struct(vec![
            ("pageUrl".into(), Type::String),
            ("n".into(), int),
            ("lang".into(), Type::Option(Box::new(Type::String))),
        ]))
    );
    assert_eq!(
        def::<Fetched>(),
        Some(Def::Enum(vec![
            ("not_found".into(), vec![]),
            ("ok".into(), vec![("0".into(), Type::String)]),
        ]))
    );
}

#[test]
fn a_shape_serde_changes_in_ways_not_modelled_is_left_unknown() {
    // Flattened fields, and a field that may be missing without being an `Option`: described
    // by name alone, so they are not checked rather than checked wrongly.
    assert_eq!(def::<Flat>(), None);
    assert_eq!(def::<Defaulted>(), None);
}
