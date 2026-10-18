//! Labels each document with its most frequent word (ties go to the alphabetically first). A
//! batch task: each step classifies all the documents given to it at once, like a model would.

use std::collections::BTreeMap;

namu_guest::batch! {
    fn classify(docs: Vec<String>) -> Vec<anyhow::Result<String>> {
        docs.iter().map(|doc| top_word(doc)).collect()
    }
}

fn top_word(doc: &str) -> anyhow::Result<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for word in doc.split_whitespace() {
        *counts.entry(word).or_default() += 1;
    }
    let (word, _) = counts
        .into_iter()
        .max_by(|(a, x), (b, y)| x.cmp(y).then(b.cmp(a)))
        .ok_or_else(|| anyhow::anyhow!("empty document"))?;
    Ok(word.to_string())
}
