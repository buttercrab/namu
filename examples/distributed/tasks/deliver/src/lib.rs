//! Delivers a document's stats to an outside system: a line appended to the file named by
//! `NAMU_DELIVERIES`, if it is set. The line carries the call's key, which a real system would
//! deduplicate on: a call the engine runs again after a crash delivers under the same key. The
//! system then takes `ack_ms` to acknowledge, so a worker that dies in that time has delivered,
//! but its branch has not moved on.
use std::io::Write;

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Stats {
    pub checksum: u64,
}

namu_guest::task! {
    fn deliver(stats: Stats, ack_ms: i64) -> anyhow::Result<i64> {
        if let Ok(path) = std::env::var("NAMU_DELIVERIES") {
            let key = namu_guest::call_key().unwrap_or_default();
            let worker = std::env::var("WORKER_ID").unwrap_or_default();
            // One write of one line: appends from every worker stay whole.
            let line = format!("delivered {key} {worker} {}\n", stats.checksum);
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?
                .write_all(line.as_bytes())?;
        }
        std::thread::sleep(std::time::Duration::from_millis(ack_ms as u64));
        Ok(1)
    }
}
