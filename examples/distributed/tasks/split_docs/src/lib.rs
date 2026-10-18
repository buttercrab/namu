//! Produces `n` synthetic documents. Its `Vec<String>` output is what the workflow fans out over.

const WORDS: &[&str] = &[
    "tree", "value", "branch", "stream", "worker", "context", "parallel", "immutable", "graph",
    "task", "node", "data", "pipeline", "model", "embed", "token", "a", "of", "to", "in",
];

namu_guest::task! {
    fn split_docs(n: i32) -> anyhow::Result<Vec<String>> {
        anyhow::ensure!(n >= 0, "n must be non-negative");
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let docs = (0..n)
            .map(|i| {
                let len = 200 + (i as usize % 7) * 150;
                (0..len)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        WORDS[(seed % WORDS.len() as u64) as usize]
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        Ok(docs)
    }
}
