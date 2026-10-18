//! Per-document analysis. Runs once per fanned-out item; `rounds` adds CPU work so the spread of
//! items across workers is visible.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Config {
    pub min_len: usize,
    pub rounds: u32,
    pub stopwords: Vec<String>,
}

#[derive(Serialize)]
pub struct Stats {
    pub words: usize,
    pub long_words: usize,
    pub top_word: String,
    pub top_count: usize,
    pub checksum: u64,
}

namu_guest::task! {
    fn analyze(doc: String, cfg: Config) -> anyhow::Result<Stats> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        let mut words = 0;
        let mut long_words = 0;
        for word in doc.split_whitespace() {
            words += 1;
            if word.len() >= cfg.min_len {
                long_words += 1;
            }
            if !cfg.stopwords.iter().any(|s| s == word) {
                *counts.entry(word).or_default() += 1;
            }
        }
        let (top_word, top_count) = counts
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(w, c)| (w.to_string(), *c))
            .unwrap_or_default();

        // Deterministic busy work (FNV-1a over the document, `rounds` times).
        let mut checksum: u64 = 0xcbf2_9ce4_8422_2325;
        for _ in 0..cfg.rounds {
            for byte in doc.bytes() {
                checksum ^= byte as u64;
                checksum = checksum.wrapping_mul(0x0100_0000_01b3);
            }
        }

        Ok(Stats { words, long_words, top_word, top_count, checksum })
    }
}
