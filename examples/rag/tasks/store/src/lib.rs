//! Upserts an embedded chunk into a Qdrant collection, at `QDRANT_URL` (default
//! `http://127.0.0.1:6333`). Its point id is the chunk's own (its article and index), so storing it
//! again, as recovery may, overwrites it: the collection never holds a chunk twice.
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub struct Chunk {
    pub doc_id: u64,
    pub title: String,
    pub url: String,
    pub index: u32,
    pub text: String,
}

#[derive(Deserialize)]
pub struct Embedded {
    pub chunk: Chunk,
    pub vector: Vec<f32>,
}

namu_guest::task! {
    fn store(embedded: Embedded, collection: String) -> anyhow::Result<i64> {
        let qdrant = std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6333".into());
        let Embedded { chunk, vector } = embedded;
        let point = json!({
            "id": (chunk.doc_id << 16) | chunk.index as u64,
            "vector": vector,
            "payload": {
                "doc_id": chunk.doc_id,
                "title": chunk.title,
                "url": chunk.url,
                "index": chunk.index,
                "text": chunk.text,
            },
        });
        ureq::put(&format!("{qdrant}/collections/{collection}/points?wait=true"))
            .send_json(json!({ "points": [point] }))
            .map_err(|err| anyhow::anyhow!("qdrant: {err}"))?;
        Ok(1)
    }
}
