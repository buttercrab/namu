use namu::prelude::*;

#[task]
fn check(x: i32) -> Result<i32>;

#[workflow]
fn plain_match() -> i32 {
    let x = 1;
    match check(x) {
        Ok(v) => v,
        Err(_) => x,
    }
}

fn main() {}
