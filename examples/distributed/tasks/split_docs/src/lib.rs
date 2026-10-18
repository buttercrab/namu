//! Produces `n` synthetic documents. Its `Vec<String>` output is what the workflow fans out over.

namu_guest::task! {
    fn split_docs(n: i32) -> anyhow::Result<Vec<String>> {
        anyhow::ensure!(n >= 0, "n must be non-negative");
        Ok(namu_example_docgen::docs().take(n as usize).collect())
    }
}
