use graph::workflow;

#[workflow]
fn immutable_assign() {
    let x = 1;
    x = 2;
}

fn main() {}
