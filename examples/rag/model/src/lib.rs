//! The RAG example's embedding model: [all-MiniLM-L6-v2], a sentence-transformers BERT model, run
//! on the CPU with candle. Texts are embedded as the mean of their tokens' last hidden states,
//! normalized to unit length, so the dot product of two embeddings is their cosine similarity.
//! The ingestion pipeline (the `embed` task) and the `query` tool share it, so a query and the
//! chunks it is compared with are embedded alike.
//!
//! [all-MiniLM-L6-v2]: https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2

use std::path::{Path, PathBuf};

use anyhow::Context;
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

pub const MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2";
/// The model's revision on Hugging Face, so every run embeds with the same weights.
pub const REVISION: &str = "1110a243fdf4706b3f48f1d95db1a4f5529b4d41";
/// The length of an embedding.
pub const DIMENSIONS: usize = 384;
/// Tokens the model reads of a text; the rest is cut off.
pub const MAX_TOKENS: usize = 256;

const FILES: [&str; 3] = ["config.json", "tokenizer.json", "model.safetensors"];

pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl Embedder {
    /// Loads the model, downloading it into `cache` first if it is not there.
    pub fn load(cache: &Path) -> anyhow::Result<Self> {
        let dir = fetch(cache)?;
        let config: Config = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)?;
        let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|err| anyhow::anyhow!("tokenizer: {err}"))?;
        tokenizer.with_padding(Some(PaddingParams::default()));
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                ..TruncationParams::default()
            }))
            .map_err(|err| anyhow::anyhow!("tokenizer: {err}"))?;
        let device = device()?;
        // SAFETY: the weights file is not modified while it is mapped.
        let weights = unsafe {
            VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], DTYPE, &device)?
        };
        let model = BertModel::load(weights, &config)?;
        Ok(Self {
            model,
            tokenizer,
            device,
        })
    }

    /// The model, with its cache directory taken from `NAMU_MODEL_CACHE`, or `data/rag/model`.
    pub fn from_env() -> anyhow::Result<Self> {
        let cache = std::env::var_os("NAMU_MODEL_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data/rag/model"));
        Self::load(&cache)
    }

    /// Embeds `texts` as one batch: unit vectors of [`DIMENSIONS`] numbers, in order.
    pub fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|err| anyhow::anyhow!("tokenizing: {err}"))?;
        let rows = |field: fn(&tokenizers::Encoding) -> &[u32]| -> anyhow::Result<Tensor> {
            let rows = encodings
                .iter()
                .map(|encoding| Tensor::new(field(encoding), &self.device))
                .collect::<candle_core::Result<Vec<_>>>()?;
            Ok(Tensor::stack(&rows, 0)?)
        };
        let ids = rows(|e| e.get_ids())?;
        let types = rows(|e| e.get_type_ids())?;
        let mask = rows(|e| e.get_attention_mask())?;
        let hidden = self.model.forward(&ids, &types, Some(&mask))?;
        // Mean over the real tokens (the mask leaves out padding), then unit length.
        let mask = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
        let summed = hidden.broadcast_mul(&mask)?.sum(1)?;
        let mean = summed.broadcast_div(&mask.sum(1)?)?;
        let norm = mean.sqr()?.sum_keepdim(1)?.sqrt()?;
        Ok(mean.broadcast_div(&norm)?.to_vec2()?)
    }
}

/// Where the model runs: the CPU, unless `NAMU_DEVICE` names a GPU (`metal`, on macOS).
fn device() -> anyhow::Result<Device> {
    match std::env::var("NAMU_DEVICE").as_deref() {
        Err(_) | Ok("cpu") => Ok(Device::Cpu),
        #[cfg(target_os = "macos")]
        Ok("metal") => Ok(Device::new_metal(0)?),
        Ok(other) => anyhow::bail!("NAMU_DEVICE={other}: no such device here"),
    }
}

/// The model's files in `cache`, downloaded at [`REVISION`] if missing. A file is written under a
/// name of its own (workers may download at once) and renamed when complete, so a download cut
/// short is never taken for one.
fn fetch(cache: &Path) -> anyhow::Result<PathBuf> {
    let dir = cache.join(MODEL.replace('/', "--")).join(REVISION);
    std::fs::create_dir_all(&dir)?;
    for file in FILES {
        let path = dir.join(file);
        if path.exists() {
            continue;
        }
        let url = format!("https://huggingface.co/{MODEL}/resolve/{REVISION}/{file}");
        let partial = dir.join(format!("{file}.{}.part", std::process::id()));
        let mut body = ureq::get(&url)
            .call()
            .with_context(|| format!("downloading {url}"))?
            .into_body();
        std::io::copy(&mut body.as_reader(), &mut std::fs::File::create(&partial)?)
            .with_context(|| format!("downloading {url}"))?;
        std::fs::rename(&partial, &path)?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn similar_sentences_are_close_and_others_are_not() {
        let embedder = Embedder::load(Path::new("../../../data/rag/model")).unwrap();
        let texts = [
            "A man is eating food.",
            "A man is eating a piece of bread.",
            "The new movie is awesome.",
        ];
        let vectors = embedder.embed(&texts).unwrap();
        assert!(vectors.iter().all(|v| v.len() == DIMENSIONS));
        assert!(vectors.iter().all(|v| (dot(v, v) - 1.0).abs() < 1e-4));
        let (close, far) = (dot(&vectors[0], &vectors[1]), dot(&vectors[0], &vectors[2]));
        assert!(close > 0.6 && far < 0.2, "close {close}, far {far}");
        // A text embeds the same alone as in a batch, padding notwithstanding.
        let alone = embedder.embed(&texts[2..]).unwrap();
        assert!(dot(&alone[0], &vectors[2]) > 0.9999);
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// `cargo test --release -- --ignored --nocapture throughput`: chunks per second, one process.
    #[test]
    #[ignore]
    fn throughput() {
        let embedder = Embedder::load(Path::new("../../../data/rag/model")).unwrap();
        let text = "word ".repeat(200);
        for batch in [1, 8, 32] {
            let texts = vec![text.as_str(); batch];
            let started = std::time::Instant::now();
            let rounds = 64 / batch;
            for _ in 0..rounds {
                embedder.embed(&texts).unwrap();
            }
            let per_second = (rounds * batch) as f64 / started.elapsed().as_secs_f64();
            println!("batch {batch:>2}: {per_second:.0} chunks/s");
        }
    }
}
