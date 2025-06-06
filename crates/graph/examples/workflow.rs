use graph::{new_literal, trace, workflow, TraceNode};

#[trace]
fn is_positive(v: i32) -> bool {
    v > 0
}

#[trace]
fn double(v: i32) -> i32 {
    v * 2
}

#[trace]
fn identity(v: i32) -> i32 {
    v
}

#[workflow]
fn my_workflow(input: TraceNode<i32>) -> TraceNode<i32> {
    if is_positive(input.clone()) {
        double(input.clone())
    } else {
        identity(input)
    }
}

fn main() {
    println!("--- Positive case (5) ---");
    let input1 = new_literal(5);
    let workflow1 = my_workflow(input1);
    println!("{}", workflow1.graph_string());
    println!("Result: {}", workflow1.run());

    println!("\n--- Negative case (-5) ---");
    let input2 = new_literal(-5);
    let workflow2 = my_workflow(input2);
    println!("{}", workflow2.graph_string());
    println!("Result: {}", workflow2.run());
}
