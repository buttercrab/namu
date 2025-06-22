use graph::workflow;
use namu as graph;

#[workflow]
fn uninitialized_simple_let() {
    let y;
}

fn main() {}
