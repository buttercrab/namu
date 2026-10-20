//! Embeds chunks with all-MiniLM-L6-v2 (`namu-rag-model`), a batch at a time: chunks from every
//! article in flight share each step, as on a GPU. The model is loaded once per worker, on its
//! first batch.
use std::sync::OnceLock;

use namu_rag_model::Embedder;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Chunk {
    pub doc_id: u64,
    pub title: String,
    pub url: String,
    pub index: u32,
    pub text: String,
}

#[derive(Serialize)]
pub struct Embedded {
    pub chunk: Chunk,
    pub vector: Vec<f32>,
}

static MODEL: OnceLock<Result<Embedder, String>> = OnceLock::new();

namu_guest::batch! {
    fn embed(chunks: Vec<Chunk>) -> Vec<anyhow::Result<Embedded>> {
        let model = match MODEL.get_or_init(|| Embedder::from_env().map_err(|err| format!("{err:#}"))) {
            Ok(model) => model,
            Err(err) => return chunks.iter().map(|_| Err(anyhow::anyhow!("{err}"))).collect(),
        };
        let texts: Vec<&str> = chunks.iter().map(|chunk| chunk.text.as_str()).collect();
        match model.embed(&texts) {
            Ok(vectors) => chunks
                .into_iter()
                .zip(vectors)
                .map(|(chunk, vector)| Ok(Embedded { chunk, vector }))
                .collect(),
            Err(err) => chunks.iter().map(|_| Err(anyhow::anyhow!("{err:#}"))).collect(),
        }
    }
}
