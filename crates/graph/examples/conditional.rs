use anyhow::Result;
use graph::{task, workflow};

#[task]
fn is_positive(v: i32) -> Result<bool> {
    Ok(v > 0)
}

#[task]
fn double(v: i32) -> Result<i32> {
    Ok(v * 2)
}

#[task]
fn is_negative(v: i32) -> Result<bool> {
    Ok(v < 0)
}

#[task]
fn identity(v: i32) -> Result<i32> {
    Ok(v)
}

#[workflow]
fn simple_workflow() -> i32 {
    let input = 10;
    if is_positive(input) {
        double(input)
    } else {
        identity(input)
    }
}

#[workflow]
fn complex_workflow() -> i32 {
    let input = 10;
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
    let input = 10;
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
    let input = 10;
    let mut b = input;
    if is_positive(input) {
        b = double(input);
    }
    double(b)
}

fn main() {
    let graph = simple_workflow();
    println!("{}", graph.graph_string());

    let graph = complex_workflow();
    println!("{}", graph.graph_string());

    let graph = complex_workflow2();
    println!("{}", graph.graph_string());

    let graph = complex_workflow3();
    println!("{}", graph.graph_string());
}
