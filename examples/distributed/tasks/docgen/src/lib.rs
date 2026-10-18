//! Synthetic documents for the examples: the same `n` documents, whether a task returns them as a
//! list or yields them as a stream.

const WORDS: &[&str] = &[
    "tree", "value", "branch", "stream", "worker", "context", "parallel", "immutable", "graph",
    "task", "node", "data", "pipeline", "model", "embed", "token", "a", "of", "to", "in",
];

/// The documents, in order. Unbounded; take as many as needed.
pub fn docs() -> impl Iterator<Item = String> {
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    (0..).map(move |i: usize| {
        let len = 200 + (i % 7) * 150;
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
}
