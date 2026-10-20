//! Fan-in over each document's number of retries.
use serde::Serialize;

#[derive(Serialize)]
pub struct RetryReport {
    pub docs: usize,
    pub retries: i64,
    pub most: i64,
}

namu_guest::task! {
    fn retry_report(retries: Vec<i64>) -> anyhow::Result<RetryReport> {
        Ok(RetryReport {
            docs: retries.len(),
            retries: retries.iter().sum(),
            most: retries.iter().copied().max().unwrap_or(0),
        })
    }
}
