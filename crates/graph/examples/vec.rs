use graph::{task, workflow};

#[task(stream)]
fn range() -> anyhow::Result<impl Iterator<Item = anyhow::Result<i32>>> {
    Ok((0..10).map(Ok))
}

#[task]
fn double(x: i32) -> anyhow::Result<i32> {
    Ok(x * 2)
}

#[task]
fn is_even(x: i32) -> anyhow::Result<bool> {
    Ok(x % 2 == 0)
}

#[workflow]
fn workflow() -> Vec<i32> {
    let xs = range();

    if is_even(xs) { double(xs) } else { xs }
}

fn main() {
    let graph = workflow();
    let graph_str = graph.graph_string();
    println!("{}", graph_str);
}
