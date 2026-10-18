//! Fan-in: receives every surviving branch's `Stats` as one list. Built as a native library.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Stats {
    pub words: usize,
    pub long_words: usize,
    pub top_word: String,
    pub top_count: usize,
    pub checksum: u64,
}

#[derive(Serialize)]
pub struct Summary {
    pub docs: usize,
    pub words: usize,
    pub long_words: usize,
    pub top_words: BTreeMap<String, usize>,
    pub checksum: u64,
}

namu_guest::task! {
    fn summarize(stats: Vec<Stats>) -> anyhow::Result<Summary> {
        let mut top_words = BTreeMap::new();
        for s in &stats {
            *top_words.entry(s.top_word.clone()).or_default() += s.top_count;
        }
        Ok(Summary {
            docs: stats.len(),
            words: stats.iter().map(|s| s.words).sum(),
            long_words: stats.iter().map(|s| s.long_words).sum(),
            top_words,
            // XOR is order-independent, so the result doesn't depend on arrival order.
            checksum: stats.iter().fold(0, |acc, s| acc ^ s.checksum),
        })
    }
}
