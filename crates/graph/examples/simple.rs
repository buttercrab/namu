use anyhow::Result;
use graph::{task, workflow};

#[task]
fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}

#[workflow]
fn workflow() -> i32 {
    let a = 1;
    let b = 2;
    add(a, b)
}

#[workflow]
fn workflow2() -> i32 {
    let a = 1;
    let mut b = 2;
    b = add(a, b);
    b = add(a, b);
    b
}

#[workflow]
fn workflow3() {
    let a = 1;
    let b = 2;
    {
        let _c = add(a, b);
    }
    let _b = 2;
}

fn main() {
    let graph = workflow();
    let graph_str = graph.graph_string();
    println!("{}", graph_str);
    let result = graph::Executor::new().run(&graph);
    println!("Result: {:?}", result);
    assert_eq!(result, 3);
}
