//! The summary of no documents: where a fold of summaries starts.
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

namu_guest::task! {
    fn empty_summary() -> anyhow::Result<Summary> {
        Ok(Summary::default())
    }
}
