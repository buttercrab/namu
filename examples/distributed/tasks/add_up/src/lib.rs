//! Fan-in: the sum of the values.
namu_guest::task! {
    fn add_up(xs: Vec<i64>) -> anyhow::Result<i64> {
        Ok(xs.iter().sum())
    }
}
