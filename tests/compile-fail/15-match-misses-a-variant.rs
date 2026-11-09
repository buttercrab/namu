use namu::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub enum Size {
    Small,
    Large(u64),
}

#[task]
fn size(n: i32) -> Result<Size>;

// A `match` in a workflow covers the enum, as one in Rust does.
#[workflow]
fn sized(n: i32) -> u64 {
    match size(n) {
        Size::Large(bytes) => bytes,
    }
}

fn main() {}
