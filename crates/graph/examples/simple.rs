use graph::{task, workflow};

#[task]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[task]
fn one() -> i32 {
    1
}

#[task]
fn two() -> i32 {
    2
}

#[workflow]
fn workflow() -> i32 {
    let a = one();
    let b = two();
    add(a, b)
}

#[workflow]
fn workflow2() -> i32 {
    let a = one();
    let mut b = two();
    b = add(a, b);
    b = add(a, b);
    b
}

#[workflow]
fn workflow3() {
    let a = one();
    let b = two();
    {
        let c = add(a, b);
    }
    let b = two();
}

fn main() {
    let graph = workflow();
    let graph_str = graph.graph_string();
    println!("{}", graph_str);
    let result = graph.run();
    println!("Result: {:?}", result);
    assert_eq!(result, 3);
}
