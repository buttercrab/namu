namu_guest::task! {
    fn less_than(a: i64, b: i64) -> anyhow::Result<bool> {
        Ok(a < b)
    }
}
