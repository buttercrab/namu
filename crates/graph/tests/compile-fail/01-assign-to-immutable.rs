use graph::workflow;

#[workflow]
fn immutable_assign() {
    let x = graph::new_literal(1);
    x = graph::new_literal(2);
}

fn main() {}
