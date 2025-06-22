use graph::workflow;
use namu as graph;

#[workflow]
fn tuple_destruct_empty() {
    let () = ();
}

fn main() {}
