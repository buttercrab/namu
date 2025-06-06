use graph::{graph_if, trace, TraceValue};

#[trace]
fn get_true() -> bool {
    true
}

#[trace]
fn get_false() -> bool {
    false
}

#[trace]
fn get_ten() -> i32 {
    10
}

#[trace]
fn get_twenty() -> i32 {
    20
}

#[trace]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn conditional_workflow(condition: bool) -> TraceValue<i32> {
    let cond = if condition { get_true() } else { get_false() };
    let ten = get_ten();
    let twenty = get_twenty();
    graph_if(cond, ten, twenty)
}

fn main() {
    println!("--- True case ---");
    let workflow_true = conditional_workflow(true);
    println!("{}", workflow_true.graph_string());
    let result_true = workflow_true.run();
    println!("Result: {}", result_true);
    assert_eq!(result_true, 10);

    println!("\n--- False case ---");
    let workflow_false = conditional_workflow(false);
    println!("{}", workflow_false.graph_string());
    let result_false = workflow_false.run();
    println!("Result: {}", result_false);
    assert_eq!(result_false, 20);
}
