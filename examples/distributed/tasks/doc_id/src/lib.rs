//! A document's id: a stable hash of its text.
namu_guest::task! {
    fn doc_id(doc: String) -> anyhow::Result<i64> {
        // FNV-1a, kept to 31 bits.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in doc.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        Ok((hash & 0x7fff_ffff) as i64)
    }
}
