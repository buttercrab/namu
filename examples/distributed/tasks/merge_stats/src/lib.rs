//! Adds one document's stats to a summary: `summarize`, one document at a time, for a reduce.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct Summary {
    pub docs: usize,
    pub words: usize,
    pub long_words: usize,
    pub top_words: BTreeMap<String, usize>,
    pub checksum: u64,
}

#[derive(Deserialize)]
pub struct Stats {
    pub words: usize,
    pub long_words: usize,
    pub top_word: String,
    pub top_count: usize,
    pub checksum: u64,
}

namu_guest::task! {
    fn merge_stats(summary: Summary, stats: Stats) -> anyhow::Result<Summary> {
        let mut summary = summary;
        summary.docs += 1;
        summary.words += stats.words;
        summary.long_words += stats.long_words;
        *summary.top_words.entry(stats.top_word).or_default() += stats.top_count;
        summary.checksum ^= stats.checksum;
        Ok(summary)
    }
}
