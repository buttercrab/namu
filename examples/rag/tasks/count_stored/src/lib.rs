//! Fan-in over an article's chunks: how many were stored.
namu_guest::task! {
    fn count_stored(stored: Vec<i64>) -> anyhow::Result<i64> {
        Ok(stored.iter().sum())
    }
}
