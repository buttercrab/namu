use namu::prelude::*;

#[task]
fn score() -> Result<f64>;

#[workflow]
fn wf() -> String {
    let s = score();
    format!("{:.2}", s)
}

fn main() {}
