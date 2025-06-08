use graph::{task, workflow};

#[task]
fn is_positive(v: i32) -> bool {
    v > 0
}

#[task]
fn double(v: i32) -> i32 {
    v * 2
}

#[task]
fn is_negative(v: i32) -> bool {
    v < 0
}

#[task]
fn identity(v: i32) -> i32 {
    v
}

#[workflow]
fn simple_workflow() -> i32 {
    let input = graph::new_literal(10);
    if is_positive(input) {
        double(input)
    } else {
        identity(input)
    }
}

#[workflow]
fn complex_workflow() -> i32 {
    let input = graph::new_literal(10);
    if is_positive(input) {
        double(input)
    } else {
        if is_negative(input) {
            double(input)
        } else {
            identity(input)
        }
    }
}

#[workflow]
fn complex_workflow2() -> i32 {
    let input = graph::new_literal(10);
    if is_positive(input) {
        double(input)
    } else if is_negative(input) {
        double(input)
    } else {
        identity(input)
    }
}

#[workflow]
fn complex_workflow3() -> i32 {
    let input = graph::new_literal(10);
    let mut b = input;
    if is_positive(input) {
        b = double(input);
    }
    double(b)
}

fn main() {
    let graph = simple_workflow();
    println!("{}", graph.graph_string());

    let result1 = graph.run::<i32>();
    println!("Result: {}", result1);
    assert_eq!(result1, 20);

    let graph = complex_workflow();
    println!("{}", graph.graph_string());

    let result2 = graph.run::<i32>();
    println!("Result: {}", result2);
    assert_eq!(result2, 20);

    let graph = complex_workflow2();
    println!("{}", graph.graph_string());

    let result3 = graph.run::<i32>();
    println!("Result: {}", result3);
    assert_eq!(result3, 20);
}
