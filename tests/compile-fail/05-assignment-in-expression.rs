use graph::workflow;
use namu as graph;

#[workflow]
fn assignment_in_expression() {
    let mut x = 1;
    let _y = (x = 2);
}

fn main() {}
