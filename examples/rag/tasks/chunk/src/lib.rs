//! Splits an article into chunks of about `words` words for embedding, each overlapping the one
//! before by a quarter, so a passage cut at a boundary is whole in one of them. The title leads
//! every chunk: it is context the text alone often lacks.
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Article {
    pub id: u64,
    pub title: String,
    pub url: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct Chunk {
    pub doc_id: u64,
    pub title: String,
    pub url: String,
    pub index: u32,
    pub text: String,
}

namu_guest::task! {
    fn chunk(article: Article, words: i32) -> anyhow::Result<Vec<Chunk>> {
        anyhow::ensure!(words >= 4, "chunks of {words} words are too small");
        let all: Vec<&str> = article.text.split_whitespace().collect();
        let (size, step) = (words as usize, words as usize * 3 / 4);
        let starts = (0..all.len().max(1)).step_by(step).take_while(|&start| {
            // The last chunk takes what is left, rather than a sliver of it.
            start == 0 || start + size / 4 < all.len()
        });
        Ok(starts
            .zip(0..)
            .map(|(start, index)| Chunk {
                doc_id: article.id,
                title: article.title.clone(),
                url: article.url.clone(),
                index,
                text: format!(
                    "{}: {}",
                    article.title,
                    all[start..(start + size).min(all.len())].join(" ")
                ),
            })
            .collect())
    }
}
