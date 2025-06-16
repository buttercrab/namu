use anyhow::Result;
use graph::{task, workflow};

#[task]
fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}

#[task]
fn less_than_10(v: i32) -> Result<bool> {
    Ok(v < 10)
}

#[workflow]
fn while_workflow() -> i32 {
    let mut i = 0;
    while less_than_10(i) {
        i = add(i, 1);
    }
    i
}

#[workflow]
fn fibonacci() -> i32 {
    let mut a = 0;
    let mut b = 1;
    let mut i = 0;

    while less_than_10(i) {
        let c = add(a, b);
        a = b;
        b = c;
        i = add(i, 1);
    }

    b
}

fn main() {
    let graph = while_workflow();
    println!("Graph: \n{}", graph.graph_string());

    let graph = fibonacci();
    println!("Graph: \n{}", graph.graph_string());
}
