//! Streams the corpus (`scripts/rag-fetch.py`): one Wikipedia article at a time, at most `limit`,
//! from the JSONL file named by `RAG_CORPUS` (default `data/rag/corpus.jsonl`).
use std::io::BufRead;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Article {
    pub id: u64,
    pub title: String,
    pub url: String,
    pub text: String,
}

namu_guest::stream! {
    fn articles(limit: i32) -> anyhow::Result<impl Iterator<Item = anyhow::Result<Article>>> {
        let path = std::env::var("RAG_CORPUS").unwrap_or_else(|_| "data/rag/corpus.jsonl".into());
        let file = std::fs::File::open(&path)
            .map_err(|err| anyhow::anyhow!("{path}: {err} (run scripts/rag-fetch.py)"))?;
        Ok(std::io::BufReader::new(file)
            .lines()
            .take(limit.max(0) as usize)
            .map(|line| Ok(serde_json::from_str(&line?)?)))
    }
}
