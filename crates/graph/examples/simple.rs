use graph::{trace, TraceNode};

#[trace]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[trace]
fn one() -> i32 {
    1
}

#[trace]
fn two() -> i32 {
    2
}

fn workflow() -> TraceNode<i32> {
    let a = one();
    let b = two();
    let c = add(a, b);
    c
}

fn main() {
    let workflow = workflow();
    let graph = workflow.graph_string();
    println!("{}", graph);
    let result = workflow.run();
    println!("Result: {:?}", result);
}
