//! Fails for documents shorter than `min_words`. Failed branches are dropped from the collect.
namu_guest::task! {
    fn reject_short(doc: String, min_words: i32) -> anyhow::Result<String> {
        let words = doc.split_whitespace().count();
        anyhow::ensure!(words >= min_words as usize, "document has only {words} words");
        Ok(doc)
    }
}
