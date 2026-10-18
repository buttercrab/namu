//! Outer fan-in over per-document summaries.
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Summary {
    pub words: usize,
}

namu_guest::task! {
    fn total_words(summaries: Vec<Summary>) -> anyhow::Result<usize> {
        Ok(summaries.iter().map(|s| s.words).sum())
    }
}
