use anyhow::Result;
use graph::{task, workflow};

// --- Tasks for nested loops ---
#[task]
fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}

#[task]
fn less_than(a: i32, b: i32) -> Result<bool> {
    Ok(a < b)
}

// --- Tasks for conditional loops ---
#[task]
fn not_one(n: i32) -> Result<bool> {
    Ok(n != 1)
}

#[task]
fn is_even(n: i32) -> Result<bool> {
    Ok(n % 2 == 0)
}

#[task]
fn divide_by_2(n: i32) -> Result<i32> {
    Ok(n / 2)
}

#[task]
fn multiply_by_3_and_add_1(n: i32) -> Result<i32> {
    Ok(n * 3 + 1)
}

/// A workflow with nested while loops to calculate 5 * 3.
#[workflow]
fn nested_loop_sum() -> i32 {
    let mut i = 0;
    let mut sum = 0;
    while less_than(i, 5) {
        let mut j = 0;
        while less_than(j, 3) {
            sum = add(sum, 1);
            j = add(j, 1);
        }
        i = add(i, 1);
    }
    sum
}

/// A workflow with a conditional inside a while loop,
/// running a Collatz-like sequence.
#[workflow]
fn conditional_loop_collatz() -> i32 {
    let mut n = 6;
    let mut count = 0;
    while not_one(n) {
        if is_even(n) {
            n = divide_by_2(n);
        } else {
            n = multiply_by_3_and_add_1(n);
        }
        count = add(count, 1);
    }
    count
}

/// A workflow where the loop condition is initially false.
#[workflow]
fn zero_iteration_loop() -> i32 {
    let mut i = 10;
    while less_than(i, 10) {
        i = add(i, 1);
    }
    i
}

/// A workflow with an `if` statement that has no `else` branch inside a loop.
#[workflow]
fn loop_with_dangling_if() -> i32 {
    let mut n = 10;
    let mut i = 0;
    while less_than(i, 5) {
        if is_even(i) {
            n = add(n, 1);
        }
        i = add(i, 1);
    }
    n
}

/// A workflow with two separate, sequential while loops.
#[workflow]
fn multiple_sequential_loops() -> i32 {
    let mut x = 0;
    while less_than(x, 5) {
        x = add(x, 1);
    }
    while less_than(x, 10) {
        x = add(x, 2);
    }
    x
}

/// A workflow where the result of a loop is used in a later conditional.
#[workflow]
fn loop_value_feeds_conditional() -> i32 {
    let mut x = 0;
    while less_than(x, 5) {
        x = add(x, 1);
    }

    if is_even(x) { 0 } else { 1 }
}

fn main() {
    println!("--- Running Nested Loop Test ---");
    let nested_loop_graph = nested_loop_sum();
    println!("Graph: \n{}", nested_loop_graph.graph_string());

    println!("\n--- Running Conditional Loop Test ---");
    let conditional_loop_graph = conditional_loop_collatz();
    println!("Graph: \n{}", conditional_loop_graph.graph_string());

    println!("\n--- Running Zero Iteration Loop Test ---");
    let zero_iter_graph = zero_iteration_loop();
    println!("Graph: \n{}", zero_iter_graph.graph_string());

    println!("\n--- Running Dangling If Loop Test ---");
    let dangling_if_graph = loop_with_dangling_if();
    println!("Graph: \n{}", dangling_if_graph.graph_string());

    println!("\n--- Running Sequential Loops Test ---");
    let sequential_loops_graph = multiple_sequential_loops();
    println!("Graph: \n{}", sequential_loops_graph.graph_string());

    println!("\n--- Running Loop Feeds Conditional Test ---");
    let loop_feeds_cond_graph = loop_value_feeds_conditional();
    println!("Graph: \n{}", loop_feeds_cond_graph.graph_string());
}
