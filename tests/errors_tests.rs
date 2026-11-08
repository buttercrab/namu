//! Typed task errors: a task that fails with a type of its own gives an attempt of it that value,
//! which the workflow can pass on, or match on by variant. A failure with no such value (a crash)
//! fails the branch. `match` works on any enum a workflow has, a task's result as well.

use std::fmt;
use std::sync::Arc;

use namu::prelude::*;
use namu_core::ir::Workflow;
use namu_engine::{FnTasks, LocalCluster, RunOutcome, Tasks};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug, Serialize, Deserialize, namu::Data)]
#[serde(rename_all = "snake_case")]
pub enum FetchError {
    NotFound,
    Moved(String),
    #[serde(rename = "limited")]
    RateLimited {
        #[serde(rename = "after")]
        retry_after: u64,
        reason: String,
    },
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, namu::Data)]
pub enum Size {
    Small,
    Large(u64),
}

#[task]
fn fetch(url: String) -> Result<String, FetchError>;

#[task]
fn fetch_small(n: i32) -> Result<String, FetchError>;

#[task]
fn wait(seconds: u64, reason: String) -> Result<String>;

#[task]
fn describe(error: FetchError) -> Result<String>;

#[task]
fn size(n: i32) -> Result<Size>;

#[task]
fn items(n: i32) -> Result<Vec<i32>>;

#[task]
fn sum(xs: Vec<i32>) -> Result<i32>;

#[task]
fn find(n: i32) -> Result<Option<String>>;

#[derive(Clone, Debug, Serialize, Deserialize, namu::Data)]
pub enum CrawlError {
    Blocked,
    BrokenAt(i32),
}

impl fmt::Display for CrawlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[task(stream)]
fn crawl(n: i32) -> Result<impl Iterator<Item = Result<i32, CrawlError>>, CrawlError>;

#[task]
fn count(pages: Vec<i32>) -> Result<i32>;

#[derive(Clone, Debug, Serialize, Deserialize, namu::Data)]
pub enum Verdict {
    TooLong(i32),
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Doubles each item a step at a time, alongside the rest of its batch; an item over 10 fails.
#[task(batch)]
fn judge(items: Vec<i32>) -> Vec<Result<i32, Verdict>>;

#[derive(Default)]
struct Judge;

impl namu_engine::tasks::Continuous for Judge {
    type In = i32;
    type Out = i32;
    type Error = Verdict;

    fn step(&mut self, admit: Vec<(u64, i32)>) -> Vec<(u64, Result<i32, Verdict>)> {
        let judged = |n: i32| {
            if n > 10 {
                Err(Verdict::TooLong(n))
            } else {
                Ok(n * 2)
            }
        };
        admit.into_iter().map(|(id, n)| (id, judged(n))).collect()
    }
}

fn tasks() -> FnTasks {
    FnTasks::new()
        .raising("fetch", |url: String| match url.as_str() {
            "/ok" => Ok("page".to_string()),
            "/old" => Err(FetchError::Moved("/new".into())),
            "/busy" => Err(FetchError::RateLimited {
                retry_after: 3,
                reason: "busy".into(),
            }),
            _ => Err(FetchError::NotFound),
        })
        // Takes a `u8`: 300 fits the signature's shape, an integer, but not the task, so the call
        // fails with only a message, as a crash does.
        .raising("fetch_small", |n: u8| {
            Err::<String, _>(FetchError::Moved(n.to_string()))
        })
        .with("wait", |(seconds, reason): (u64, String)| {
            Ok::<_, String>(format!("waited {seconds}s: {reason}"))
        })
        .with("describe", |error: FetchError| {
            Ok::<_, String>(format!("failed: {error}"))
        })
        // `n` pages, unless `n` is negative (it will not open) or above 5 (page 5 is broken).
        .raising_stream("crawl", |n: i32| {
            if n < 0 {
                return Err(CrawlError::Blocked);
            }
            Ok((1..=n).map(|page| match page {
                5 => Err(CrawlError::BrokenAt(5)),
                page => Ok(page),
            }))
        })
        .with_continuous("judge", Judge::default)
        .with("items", |n: i32| {
            Ok::<_, String>((1..=n).collect::<Vec<_>>())
        })
        .with("sum", |xs: Vec<i32>| {
            Ok::<_, String>(xs.iter().sum::<i32>())
        })
        .with("count", |pages: Vec<i32>| {
            Ok::<_, String>(pages.len() as i32)
        })
        .with("find", |n: i32| {
            Ok::<_, String>((n > 0).then(|| format!("page {n}")))
        })
        .with("size", |n: i32| {
            Ok::<_, String>(if n < 10 {
                Size::Small
            } else {
                Size::Large(n as u64)
            })
        })
}

async fn run(workflow: Workflow, inputs: serde_json::Value, tasks: FnTasks) -> RunOutcome {
    let serde_json::Value::Object(inputs) = inputs else {
        panic!("inputs are an object")
    };
    let tasks: Arc<dyn Tasks> = Arc::new(tasks);
    LocalCluster::uniform(3, tasks)
        .run_with(workflow, namu::Inputs::from_json(inputs))
        .await
        .unwrap()
}

fn results(outcome: &RunOutcome) -> Vec<serde_json::Value> {
    let values = outcome.results.iter();
    values
        .map(|value| rmp_serde::from_slice(&value.encode().unwrap()).unwrap())
        .collect()
}

#[workflow]
fn handled(url: String) -> String {
    match attempt(fetch(url)) {
        Ok(page) => page,
        Err(FetchError::NotFound) => "missing",
        Err(FetchError::Moved(to)) => to,
        Err(FetchError::RateLimited {
            retry_after,
            reason,
        }) => wait(retry_after, reason),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_error_is_matched_on_by_its_variant() {
    for (url, expected) in [
        ("/ok", "page"),
        ("/gone", "missing"),
        ("/old", "/new"),
        ("/busy", "waited 3s: busy"),
    ] {
        let outcome = run(
            handled().to_serializable("handled".into()),
            json!({ "url": url }),
            tasks(),
        )
        .await;
        assert_eq!(results(&outcome), [json!(expected)], "{url}");
    }
}

#[workflow]
fn passed_on(url: String) -> String {
    match attempt(fetch(url)) {
        Ok(page) => page,
        Err(error) => describe(error),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_error_is_a_value_of_its_type() {
    let workflow = passed_on().to_serializable("passed_on".into());
    let outcome = run(workflow, json!({ "url": "/old" }), tasks()).await;
    assert_eq!(results(&outcome), [json!("failed: Moved(\"/new\")")]);
}

#[workflow]
fn crashes() -> String {
    match attempt(fetch_small(300)) {
        Ok(page) => page,
        Err(error) => describe(error),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failure_with_no_error_value_fails_the_branch() {
    let outcome = run(
        crashes().to_serializable("crashes".into()),
        json!({}),
        tasks(),
    )
    .await;
    assert!(outcome.results.is_empty());
    let [failure] = &outcome.failures[..] else {
        panic!("one failure: {:?}", outcome.failures)
    };
    assert!(failure.contains("fetch_small: bad input"), "{failure}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_error_type_is_checked_against_the_task_s() {
    // `fetch` as built, failing with messages: a workflow that catches its errors as
    // `FetchError`s is refused before it runs.
    let tasks = tasks().with("fetch", |url: String| Err::<String, _>(format!("no {url}")));
    let tasks: Arc<dyn Tasks> = Arc::new(tasks);
    let inputs = json!({ "url": "/ok" }).as_object().unwrap().clone();
    let refused = LocalCluster::uniform(1, tasks)
        .run_with(
            handled().to_serializable("handled".into()),
            namu::Inputs::from_json(inputs),
        )
        .await
        .unwrap_err();
    let refused = format!("{refused:#}");
    assert!(
        refused.contains("call to `fetch` (op 0): error: `String` is read as"),
        "{refused}"
    );
}

#[workflow]
fn sized(n: i32) -> u64 {
    match size(n) {
        Size::Small => 0,
        Size::Large(bytes) => bytes,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn any_enum_is_matched_on() {
    for (n, expected) in [(3, 0), (40, 40)] {
        let outcome = run(
            sized().to_serializable("sized".into()),
            json!({ "n": n }),
            tasks(),
        )
        .await;
        assert_eq!(results(&outcome), [json!(expected)], "{n}");
    }
}

#[workflow]
fn sized_or_not(n: i32) -> u64 {
    match size(n) {
        Size::Large(bytes) => bytes,
        _ => 1,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_arm_may_match_anything_left() {
    let outcome = run(
        sized_or_not().to_serializable("sized".into()),
        json!({ "n": 3 }),
        tasks(),
    )
    .await;
    assert_eq!(results(&outcome), [json!(1)]);
}

/// A size as a task written elsewhere gives it: by name, with nothing said of its shape, so the
/// signature check cannot tell it from a `Size`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct Tag(String);

impl namu::Describe for Tag {}
impl namu::Data for Tag {}

#[workflow]
fn small_last(n: i32) -> u64 {
    match size(n) {
        Size::Large(bytes) => bytes,
        Size::Small => 0,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_variant_no_arm_matches_fails_the_branch() {
    let sizes =
        |tag: &'static str| tasks().with("size", move |_: i32| Ok::<_, String>(Tag(tag.into())));
    let outcome = run(
        small_last().to_serializable("small_last".into()),
        json!({ "n": 3 }),
        sizes("Huge"),
    )
    .await;
    assert!(outcome.results.is_empty());
    let [failure] = &outcome.failures[..] else {
        panic!("one failure: {:?}", outcome.failures)
    };
    assert!(
        failure.contains("the variant `Huge` matches no arm"),
        "{failure}"
    );
    // And one that is the last arm's variant takes it.
    let outcome = run(
        small_last().to_serializable("small_last".into()),
        json!({ "n": 3 }),
        sizes("Small"),
    )
    .await;
    assert_eq!(results(&outcome), [json!(0)]);
}

#[workflow]
fn found(n: i32) -> String {
    match find(n) {
        Some(page) => page,
        None => "nothing",
    }
}

#[workflow]
fn found_none_first(n: i32) -> String {
    match find(n) {
        None => "nothing",
        Some(page) => page,
    }
}

#[workflow]
fn found_or_not(n: i32) -> String {
    match find(n) {
        Some(_) => "something",
        _ => "nothing",
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_option_is_matched_on() {
    for (n, expected) in [(2, "page 2"), (0, "nothing")] {
        for workflow in [found(), found_none_first()] {
            let workflow = workflow.to_serializable("found".into());
            let outcome = run(workflow, json!({ "n": n }), tasks()).await;
            assert_eq!(results(&outcome), [json!(expected)], "{n}");
        }
    }
    let outcome = run(
        found_or_not().to_serializable("found".into()),
        json!({ "n": 2 }),
        tasks(),
    )
    .await;
    assert_eq!(results(&outcome), [json!("something")]);
}

#[workflow]
fn crawled(n: i32) -> i32 {
    match attempt(crawl(n)) {
        Ok(pages) => count(pages),
        Err(CrawlError::Blocked) => -1,
        Err(CrawlError::BrokenAt(page)) => page * 100,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_fails_with_its_own_error_opening_or_at_an_item() {
    for (n, expected) in [(3, 3), (-1, -1), (9, 500)] {
        let workflow = crawled().to_serializable("crawled".into());
        let outcome = run(workflow, json!({ "n": n }), tasks()).await;
        assert_eq!(results(&outcome), [json!(expected)], "{n}");
    }
}

#[workflow]
fn judged() -> i32 {
    let n = each(items(12));
    let verdict = match attempt(judge(n)) {
        Ok(doubled) => doubled,
        Err(Verdict::TooLong(n)) => 0 - n,
    };
    sum(verdict)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_item_fails_with_its_own_error() {
    let outcome = run(
        judged().to_serializable("judged".into()),
        json!({}),
        tasks(),
    )
    .await;
    // 1..=10 doubled, then -11 and -12.
    assert_eq!(results(&outcome), [json!(110 - 23)]);
}
