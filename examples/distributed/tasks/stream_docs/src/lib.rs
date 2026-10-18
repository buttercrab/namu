//! Yields the same documents as `split_docs`, one every `delay_ms`, like a crawler would. A
//! workflow fans out over them as they arrive.

namu_guest::stream! {
    fn stream_docs(n: i32, delay_ms: i32) -> anyhow::Result<impl Iterator<Item = anyhow::Result<String>>> {
        anyhow::ensure!(n >= 0 && delay_ms >= 0, "n and delay_ms must be non-negative");
        let delay = std::time::Duration::from_millis(delay_ms as u64);
        Ok(namu_example_docgen::docs().take(n as usize).map(move |doc| {
            std::thread::sleep(delay);
            Ok(doc)
        }))
    }
}
