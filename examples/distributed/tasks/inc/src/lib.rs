//! Trivial task for measuring per-step framework overhead (WASM).
namu_guest::task! {
    fn inc(x: i64) -> anyhow::Result<i64> {
        Ok(x + 1)
    }
}
