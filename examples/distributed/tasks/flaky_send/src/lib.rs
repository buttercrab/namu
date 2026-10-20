//! Sends a document to a flaky service, which turns down the first `id % 4` attempts for it
//! (deterministically, so a run's retries are known). Returns a receipt: the id.
namu_guest::task! {
    fn flaky_send(id: i64, attempt: i64) -> anyhow::Result<i64> {
        if attempt < id % 4 {
            anyhow::bail!("service unavailable (attempt {attempt})");
        }
        Ok(id)
    }
}
