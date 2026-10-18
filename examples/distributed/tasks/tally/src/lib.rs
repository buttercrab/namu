//! Counts how many documents got each label.

use std::collections::BTreeMap;

namu_guest::task! {
    fn tally(labels: Vec<String>) -> anyhow::Result<BTreeMap<String, usize>> {
        let mut counts = BTreeMap::new();
        for label in labels {
            *counts.entry(label).or_default() += 1;
        }
        Ok(counts)
    }
}
