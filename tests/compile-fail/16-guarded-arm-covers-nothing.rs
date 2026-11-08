use namu::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub enum Size {
    Small,
    Large(u64),
}

#[task]
fn size(n: i32) -> Result<Size>;

// A guard may not hold, so its arm covers nothing, as in Rust.
#[workflow]
fn sized(n: i32) -> u64 {
    match size(n) {
        Size::Large(bytes) if bytes > 100 => bytes,
        Size::Small => 0,
    }
}

fn main() {}
