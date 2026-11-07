use namu::prelude::*;

#[task]
fn add(a: i32, b: i32) -> Result<i32>;

#[workflow]
fn sum_pair((a, b): (i32, i32)) -> i32 {
    add(a, b)
}

fn main() {}
