use graph::{trace, workflow, TraceValue};

#[trace]
fn range() -> Vec<i32> {
    (0..10).collect()
}

#[trace]
fn double(x: i32) -> i32 {
    x * 2
}

#[trace]
fn is_even(x: i32) -> bool {
    x % 2 == 0
}

#[workflow]
fn workflow() -> TraceValue<Vec<i32>> {
    let xs = range();

    if is_even(xs) {
        double(xs)
    } else {
        xs
    }
}

fn main() {}
