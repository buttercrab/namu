//! The RAG example's ingestion workflow: Wikipedia articles into a vector index.
//!
//! The tasks are declared as signatures only; they live in `../tasks`.

#![allow(dead_code)]

use namu::prelude::*;

pub struct Article;
pub struct Chunk;
pub struct Embedded;
pub struct IndexReport;

/// Streams the corpus, one article at a time.
#[task(stream)]
pub fn articles(limit: i32) -> Result<impl Iterator<Item = Result<Article>>>;

/// Splits an article into overlapping chunks of about `words` words.
#[task(single)]
pub fn chunk(article: Article, words: i32) -> Result<Vec<Chunk>>;

/// Embeds chunks with all-MiniLM-L6-v2, a batch at a time.
#[task(batch)]
pub fn embed(chunks: Vec<Chunk>) -> Vec<Result<Embedded>>;

/// Upserts an embedded chunk into a Qdrant collection.
#[task(single)]
pub fn store(embedded: Embedded, collection: String) -> Result<i64>;

#[task(single)]
pub fn count_stored(stored: Vec<i64>) -> Result<i64>;

#[task(single)]
pub fn index_report(chunks_per_article: Vec<i64>) -> Result<IndexReport>;

/// Indexes Wikipedia's machine learning articles for semantic search.
///
/// - Articles stream in, and each is chunked as it arrives (a branch per article).
/// - Every chunk is a branch of its own; `embed` batches chunks across all articles in flight.
/// - Chunks are upserted by their own ids, so recovery never stores one twice.
/// - Two fan-ins count the chunks per article, then the articles.
#[workflow(id = "index_wikipedia")]
pub fn index_wikipedia() -> IndexReport {
    let limit = 300;
    let words = 200;
    let collection = "wikipedia";
    let article = articles(limit);
    let chunks = chunk(article, words);
    let embedded = embed(chunks);
    let stored = store(embedded, collection);
    let per_article = count_stored(stored);
    index_report(per_article)
}
