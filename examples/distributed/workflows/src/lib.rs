//! Workflows for the distribution proof of concept.
//!
//! The tasks are declared as signatures only: their implementations live in `../tasks` and are
//! built as WASM components or native libraries. The workflow crate needs only the types, so
//! rustc can check every call, and it resolves fan-out and fan-in from them.

#![allow(dead_code)]

use namu::prelude::*;
use serde::{Deserialize, Serialize};

// The values that pass between tasks, as the tasks (in `../tasks`) define them. The IR describes
// every value by these shapes.

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Config {
    pub min_len: usize,
    pub rounds: u32,
    pub stopwords: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Stats {
    pub words: usize,
    pub long_words: usize,
    pub top_word: String,
    pub top_count: usize,
    pub checksum: u64,
}

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Summary {
    pub docs: usize,
    pub words: usize,
    pub long_words: usize,
    pub top_words: std::collections::BTreeMap<String, usize>,
    pub checksum: u64,
}

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct RetryReport {
    pub docs: usize,
    pub retries: i64,
    pub most: i64,
}

#[task(single)]
pub fn split_docs(n: i32) -> Result<Vec<String>>;

#[task(single)]
pub fn make_config(min_len: i32, rounds: i32) -> Result<Config>;

#[task(single)]
pub fn analyze(doc: String, cfg: Config) -> Result<Stats>;

#[task(single)]
pub fn summarize(stats: Vec<Stats>) -> Result<Summary>;

#[task(single)]
pub fn inc(x: i64) -> Result<i64>;

#[task(single)]
pub fn inc_native(x: i64) -> Result<i64>;

#[task(single)]
pub fn less_than(a: i64, b: i64) -> Result<bool>;

/// fan-out -> per-item work -> fan-in, across machines.
///
/// - `cfg` is produced once in the root context; every branch reads it through the value tree,
///   wherever it runs.
/// - `analyze` takes a `String` but receives `Vec<String>`: one branch per document (WASM).
/// - `summarize` takes `Vec<Stats>` but receives per-item `Stats`: collect (native).
#[workflow(id = "doc_stats")]
pub fn doc_stats() -> Summary {
    let min_len = 6;
    let rounds = 4000;
    let n = 48;
    let cfg = make_config(min_len, rounds);
    let docs = split_docs(n);
    let stats = analyze(docs, cfg);
    summarize(stats)
}

/// Sequential loop of trivial WASM calls on one worker: measures per-step overhead.
#[workflow(id = "bench_local")]
pub fn bench_local() -> i64 {
    let n = 1000;
    let mut i = 0;
    while less_than(i, n) {
        i = inc(i);
    }
    i
}

/// Alternates WASM and native calls. With WASM and native on different workers, every
/// iteration hands the context to the other machine and back: measures handoff overhead.
#[workflow(id = "bench_handoff")]
pub fn bench_handoff() -> i64 {
    let n = 200;
    let mut i = 0;
    while less_than(i, n) {
        i = inc(i);
        i = inc_native(i);
    }
    i
}

#[task(single)]
pub fn chunk(doc: String, size: i32) -> Result<Vec<String>>;

#[task(single)]
pub fn reject_short(doc: String, min_words: i32) -> Result<String>;

#[task(single)]
pub fn total_words(summaries: Vec<Summary>) -> Result<i64>;

/// Nested fan-out: documents, then chunks of each document. The inner collect closes the chunk
/// scope (one summary per document); the outer collect closes the document scope.
#[workflow(id = "nested_stats")]
pub fn nested_stats() -> i64 {
    let min_len = 6;
    let rounds = 1;
    let n = 48;
    let size = 64;
    let cfg = make_config(min_len, rounds);
    let docs = split_docs(n);
    let chunks = chunk(docs, size); // fan-out over docs
    let stats = analyze(chunks, cfg); // fan-out over each doc's chunks
    let per_doc = summarize(stats); // collect chunks -> one summary per doc
    total_words(per_doc) // collect docs
}

/// Branches that fail are dropped from the collect; the run ends as `partial_failed`.
#[workflow(id = "doc_stats_partial")]
pub fn doc_stats_partial() -> Summary {
    let min_len = 6;
    let rounds = 1;
    let n = 48;
    let min_words = 500;
    let cfg = make_config(min_len, rounds);
    let docs = split_docs(n);
    let long_docs = reject_short(docs, min_words);
    let stats = analyze(long_docs, cfg);
    summarize(stats)
}

/// `doc_stats` with much more work per document, so a run lasts long enough to kill a worker
/// in the middle of it (see `scripts/e2e.sh`). Recovery must produce the same result.
#[workflow(id = "doc_stats_slow")]
pub fn doc_stats_slow() -> Summary {
    let min_len = 6;
    let rounds = 150000;
    let n = 48;
    let cfg = make_config(min_len, rounds);
    let docs = split_docs(n);
    let stats = analyze(docs, cfg);
    summarize(stats)
}

#[task(single)]
pub fn abort_short(doc: String, min_words: i32) -> Result<String>;

/// Short documents crash `abort_short` on every attempt. Workers retry them, then fail those
/// branches; the run ends `partial_failed` with the long documents summarized.
#[workflow(id = "doc_stats_poison")]
pub fn doc_stats_poison() -> Summary {
    let min_len = 6;
    let rounds = 1;
    let n = 48;
    let min_words = 500;
    let cfg = make_config(min_len, rounds);
    let docs = split_docs(n);
    let long_docs = abort_short(docs, min_words);
    let stats = analyze(long_docs, cfg);
    summarize(stats)
}

/// The same documents as `split_docs`, yielded one at a time, like a crawler.
#[task(stream)]
pub fn stream_docs(n: i32, delay_ms: i32) -> Result<impl Iterator<Item = Result<String>>>;

/// `doc_stats` with the documents streamed in: each is analyzed as soon as it arrives, while the
/// rest are still coming. Same result as `doc_stats`.
#[workflow(id = "stream_stats")]
pub fn stream_stats() -> Summary {
    let min_len = 6;
    let rounds = 4000;
    let n = 48;
    let delay_ms = 5;
    let cfg = make_config(min_len, rounds);
    let docs = stream_docs(n, delay_ms);
    let stats = analyze(docs, cfg);
    summarize(stats)
}

/// A document's most frequent word. A batch task: documents from every branch (and run) are
/// classified together, a step at a time.
#[task(batch)]
pub fn classify(docs: Vec<String>) -> Vec<Result<String>>;

/// A label, refined one letter per step. Continuous batching: a short word never waits for a
/// long one.
#[task(batch)]
pub fn refine(labels: Vec<String>) -> Vec<Result<String>>;

#[task(single)]
pub fn tally(labels: Vec<String>) -> Result<std::collections::BTreeMap<String, usize>>;

/// Labels the documents with two batch tasks, then counts the labels.
#[workflow(id = "label_stats")]
pub fn label_stats() -> std::collections::BTreeMap<String, usize> {
    let n = 48;
    let docs = split_docs(n);
    let topics = classify(docs);
    let labels = refine(topics);
    tally(labels)
}

#[task(single)]
pub fn doc_id(doc: String) -> Result<i64>;

/// Turns down the first `id % 4` attempts for a document.
#[task(single)]
pub fn flaky_send(id: i64, attempt: i64) -> Result<i64>;

#[task(single)]
pub fn retry_report(retries: Vec<i64>) -> Result<RetryReport>;

/// Errors as values: every document goes to a flaky service, and its branch tries again until a
/// send goes through (up to 5 tries). A failed send is matched on, not a failed branch.
#[workflow(id = "retry_stats")]
pub fn retry_stats() -> RetryReport {
    let n = 48;
    let docs = split_docs(n);
    let id = doc_id(docs);
    let mut tries = 0;
    let mut retries = 0;
    while tries < 5 {
        match attempt(flaky_send(id, tries)) {
            Ok(_) => break,
            Err(_) => retries += 1,
        }
        tries += 1;
    }
    retry_report(retries)
}

/// Delivers each document's stats to an outside system, keyed by the call, which takes `ack_ms`
/// to acknowledge.
#[task(single)]
pub fn deliver(stats: Stats, ack_ms: i64) -> Result<i64>;

#[task(single)]
pub fn add_up(xs: Vec<i64>) -> Result<i64>;

/// Delivers every document's stats as a side effect, which is slow to acknowledge.
/// `scripts/e2e.sh` kills a worker during it, which has delivered without moving on: the engine
/// runs those deliveries again, under the keys they had, so the outside system sees each
/// document's delivery under exactly one key.
#[workflow(id = "deliveries_slow")]
pub fn deliveries_slow() -> i64 {
    let min_len = 6;
    let rounds = 1;
    let n = 48;
    let ack_ms = 500;
    let cfg = make_config(min_len, rounds);
    let docs = split_docs(n);
    let stats = analyze(docs, cfg);
    let delivered = deliver(stats, ack_ms);
    add_up(delivered)
}

#[task(single)]
pub fn empty_summary() -> Result<Summary>;

/// Adds one document's stats to a summary.
#[task(reduce)]
pub fn merge_stats(summary: Summary, stats: Stats) -> Result<Summary>;

/// `doc_stats`, reduced: each document's stats are folded into the summary as they come, in
/// document order, instead of collected into a list first. The same result.
#[workflow(id = "doc_stats_reduced")]
pub fn doc_stats_reduced() -> Summary {
    let min_len = 6;
    let rounds = 4000;
    let n = 48;
    let cfg = make_config(min_len, rounds);
    let empty = empty_summary();
    let docs = split_docs(n);
    let stats = analyze(docs, cfg);
    merge_stats(empty, stats)
}

/// Windows over a stream: the documents arrive one by one, are analyzed as they come, and every 8
/// of them are summarized together (a window), as a batch job over an endless feed would; the
/// windows' summaries are then totalled.
#[workflow(id = "stream_windows")]
pub fn stream_windows() -> i64 {
    let min_len = 6;
    let rounds = 1;
    let n = 48;
    let delay_ms = 1;
    let cfg = make_config(min_len, rounds);
    let doc = each(stream_docs(n, delay_ms));
    let stats = analyze(doc, cfg);
    let batch = window(stats, 8);
    let summary = summarize(batch);
    total_words(summary)
}
