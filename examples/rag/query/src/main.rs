//! Asks the index a question: embeds it with the model the pipeline used, and prints the chunks
//! nearest to it in the Qdrant collection.
//!
//! ```text
//! query [--top N] [--collection NAME] QUESTION...
//! query --warm        # only download the model
//! ```
//!
//! Qdrant is at `QDRANT_URL` (default `http://127.0.0.1:6333`); the model is cached in
//! `NAMU_MODEL_CACHE` (default `data/rag/model`).

use anyhow::Context;
use namu_rag_model::Embedder;
use serde_json::{Value, json};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (mut top, mut collection, mut words) = (5, "wikipedia".to_string(), Vec::new());
    let mut warm = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--top" => top = args.next().context("--top N")?.parse()?,
            "--collection" => collection = args.next().context("--collection NAME")?,
            "--warm" => warm = true,
            _ => words.push(arg),
        }
    }
    let embedder = Embedder::from_env()?;
    if warm {
        return Ok(());
    }
    anyhow::ensure!(!words.is_empty(), "usage: query [--top N] QUESTION...");
    let question = words.join(" ");
    let vector = embedder.embed(&[&question])?.remove(0);

    let qdrant = std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6333".into());
    let found: Value = ureq::post(&format!("{qdrant}/collections/{collection}/points/search"))
        .send_json(json!({ "vector": vector, "limit": top, "with_payload": true }))?
        .body_mut()
        .read_json()?;
    println!("? {question}");
    for hit in found["result"].as_array().context("no results")? {
        let payload = &hit["payload"];
        let text = payload["text"].as_str().unwrap_or_default();
        let snippet: String = text
            .split_once(": ")
            .map_or(text, |(_, rest)| rest)
            .chars()
            .take(160)
            .collect();
        println!(
            "  {:.3}  {} (chunk {})  {}\n         {snippet}…",
            hit["score"].as_f64().unwrap_or_default(),
            payload["title"].as_str().unwrap_or_default(),
            payload["index"],
            payload["url"].as_str().unwrap_or_default(),
        );
    }
    Ok(())
}
