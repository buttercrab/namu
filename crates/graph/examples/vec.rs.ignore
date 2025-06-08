use graph::{task, workflow};

#[task]
fn range() -> Vec<i32> {
    (0..10).collect()
}

#[task]
fn double(x: i32) -> i32 {
    x * 2
}

#[task]
fn is_even(x: i32) -> bool {
    x % 2 == 0
}

#[workflow]
fn workflow() -> graph::TracedValue<Vec<i32>> {
    let xs = range();

    if is_even(xs) { double(xs) } else { xs }
}

fn main() {
    let graph = workflow();
    let graph_str = graph.graph_string();
    println!("{}", graph_str);
    let result = graph.run::<Vec<i32>>();
    println!("Result: {:?}", result);
    let expected: Vec<i32> = (0..10)
        .map(|x| if x % 2 == 0 { x * 2 } else { x })
        .collect();
    assert_eq!(result, expected);
}
