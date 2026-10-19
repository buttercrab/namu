use namu::prelude::*;

#[task]
fn check(x: i32) -> Result<i32>;

#[workflow]
fn unmatched() {
    let x = 1;
    let r = attempt(check(x));
}

fn main() {}
