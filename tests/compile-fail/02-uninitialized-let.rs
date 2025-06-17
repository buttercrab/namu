use graph::workflow;

#[workflow]
fn uninitialized_let() {
    let x: graph::TracedValue<i32>;
}

fn main() {}
