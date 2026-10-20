//! Fan-in over the articles: how many were indexed, in how many chunks.
use serde::Serialize;

#[derive(Serialize)]
pub struct IndexReport {
    pub articles: usize,
    pub chunks: i64,
}

namu_guest::task! {
    fn index_report(chunks_per_article: Vec<i64>) -> anyhow::Result<IndexReport> {
        Ok(IndexReport {
            articles: chunks_per_article.len(),
            chunks: chunks_per_article.iter().sum(),
        })
    }
}
