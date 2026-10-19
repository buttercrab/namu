use namu::prelude::*;

#[task]
fn check(x: i32) -> Result<bool>;

#[workflow]
fn labeled() {
    let x = 1;
    'outer: while check(x) {
        break 'outer;
    }
}

fn main() {}
