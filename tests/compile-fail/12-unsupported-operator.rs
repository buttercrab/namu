use namu::prelude::*;

#[workflow]
fn bits() -> i32 {
    let a = 6;
    let b = 3;
    a & b
}

fn main() {}
