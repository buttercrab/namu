//! Trivial task for measuring per-step framework overhead (native).
namu_guest::task! {
    fn inc_native(x: i64) -> anyhow::Result<i64> {
        Ok(x + 1)
    }
}
