use namu::prelude::*;

#[task]
fn check(x: i32) -> Result<bool>;

#[workflow]
fn stray() {
    let x = 1;
    if check(x) {
        break;
    }
}

fn main() {}
