//! Aborts (a WASM trap, not a task error) for documents shorter than `min_words`. Every attempt
//! is an infrastructure failure, so these items are retried and then failed for good.
namu_guest::task! {
    fn abort_short(doc: String, min_words: i32) -> anyhow::Result<String> {
        if doc.split_whitespace().count() < min_words as usize {
            std::process::abort();
        }
        Ok(doc)
    }
}
