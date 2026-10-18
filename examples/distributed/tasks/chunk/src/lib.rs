//! Splits a document into chunks of `size` words: a fan-out inside a fan-out.
namu_guest::task! {
    fn chunk(doc: String, size: i32) -> anyhow::Result<Vec<String>> {
        anyhow::ensure!(size > 0, "size must be positive");
        let words: Vec<&str> = doc.split_whitespace().collect();
        Ok(words.chunks(size as usize).map(|c| c.join(" ")).collect())
    }
}
