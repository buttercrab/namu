// Allow dead code for workflow functions that are used in tests.
#![allow(dead_code)]

mod common;

use crate::common::*;
use graph::workflow;

#[test]
fn simple_workflow_run() {
    #[workflow]
    fn test_workflow() -> i32 {
        let a = 1;
        let b = 2;
        add(a, b)
    }

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 3);
}

#[test]
fn reassignment_workflow_run() {
    #[workflow]
    fn test_workflow() -> i32 {
        let a = 1;
        let mut b = 2;
        b = add(a, b); // b is now 3
        b = add(a, b); // b is now 4
        b
    }

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 4);
}

#[test]
fn conditional_workflow_run() {
    #[workflow]
    fn test_workflow() -> i32 {
        let input = 10;
        if is_positive(input) {
            double(input)
        } else {
            identity(input)
        }
    }

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 20);
}

#[test]
fn nested_conditional_workflow_run() {
    #[workflow]
    fn test_workflow() -> i32 {
        let input = 10;
        if is_negative(input) {
            double(input)
        } else {
            if is_positive(input) {
                double(input)
            } else {
                identity(input)
            }
        }
    }

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 20);
}

#[test]
fn mutable_var_in_conditional_branch_run() {
    #[workflow]
    fn test_workflow() -> i32 {
        let mut val = 10;
        if is_positive(val) {
            val = double(val);
        }
        val
    }

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 20);
}

#[test]
fn conditional_execution_does_not_execute_un_taken_branch() {
    #[workflow]
    fn test_workflow() -> i32 {
        let input = 10;
        if is_positive(input) {
            double(input)
        } else {
            panicker()
        }
    }

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 20);
}

#[test]
fn mutable_var_updated_in_conditional_and_used_after() {
    #[workflow]
    fn test_workflow() -> i32 {
        let input = 10;
        let mut b = 0;
        if is_positive(input) {
            b = double(input);
        } else {
            let a = identity(input);
            b = a;
        };
        add(b, b)
    }

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 40);
}

#[test]
fn while_loop_run() {
    #[workflow]
    fn test_workflow() -> i32 {
        let mut i = 0;
        while less_than(i, 10) {
            i = add(i, 1);
        }
        i
    }

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 10);
}

#[test]
fn nested_while_loop_run() {
    #[workflow]
    fn test_workflow() -> i32 {
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

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 15);
}

#[test]
fn conditional_in_while_loop_run() {
    #[workflow]
    fn test_workflow() -> i32 {
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

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 8);
}

#[test]
fn zero_iteration_while_loop_run() {
    #[workflow]
    fn test_workflow() -> i32 {
        let mut i = 10;
        while less_than(i, 10) {
            i = add(i, 1);
        }
        i
    }

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 10);
}

#[test]
fn dangling_if_in_while_loop_run() {
    #[workflow]
    fn test_workflow() -> i32 {
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

    let graph = test_workflow();
    assert_eq!(graph::Executor::new().run(&graph), 13);
}
