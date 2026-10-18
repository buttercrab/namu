//! Builds the analysis config once, in the parent context. Every fanned-out branch reads this
//! value through the value tree, even when the branch runs on another machine.

use serde::Serialize;

#[derive(Serialize)]
pub struct Config {
    pub min_len: usize,
    pub rounds: u32,
    pub stopwords: Vec<String>,
}

namu_guest::task! {
    fn make_config(min_len: i32, rounds: i32) -> anyhow::Result<Config> {
        Ok(Config {
            min_len: min_len.max(0) as usize,
            rounds: rounds.max(1) as u32,
            stopwords: ["a", "of", "to", "in"].iter().map(|s| s.to_string()).collect(),
        })
    }
}
