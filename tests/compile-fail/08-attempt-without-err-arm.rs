use namu::prelude::*;

#[task]
fn check(x: i32) -> Result<i32>;

#[workflow]
fn one_arm() -> i32 {
    let x = 1;
    match attempt(check(x)) {
        Ok(v) => v,
        Ok(w) => w,
    }
}

fn main() {}
