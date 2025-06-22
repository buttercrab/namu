use graph::workflow;
use namu as graph;

#[workflow]
fn uninitialized_let() {
    let x: graph::TracedValue<i32>;
}

fn main() {}
