#![allow(dead_code)]

mod common;

use crate::common::*;
use graph::workflow;

#[test]
fn simple_graph_structure() {
    #[workflow]
    fn test_workflow() -> i32 {
        let a = 1;
        let b = 2;
        add(a, b)
    }

    let graph = test_workflow();
    let expected = r#"Block 0:
  let var0 = 1;
  let var1 = 2;
  let var2 = add(var0, var1);
  return var2"#;
    assert_eq!(graph.graph_string().trim(), expected.trim());
}

#[test]
fn conditional_graph_structure() {
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
    let expected_graph = "Block 0:
  let var0 = 10;
  let var1 = is_positive(var0);
  branch var1 ? Block 2 : Block 3
Block 1:
  let var4 = phi([block 2, var2], [block 3, var3]);
  return var4
Block 2:
  let var2 = double(var0);
  jump -> Block 1
Block 3:
  let var3 = identity(var0);
  jump -> Block 1";
    assert_eq!(graph.graph_string().trim(), expected_graph.trim());
}

#[test]
fn nested_conditional_graph_structure() {
    #[workflow]
    fn test_workflow() -> i32 {
        let input = 10;
        if is_positive(input) {
            double(input)
        } else if is_negative(input) {
            double(input)
        } else {
            identity(input)
        }
    }

    let graph = test_workflow();
    let expected_graph = "Block 0:
  let var0 = 10;
  let var1 = is_positive(var0);
  branch var1 ? Block 2 : Block 3
Block 1:
  let var7 = phi([block 2, var2], [block 4, var6]);
  return var7
Block 2:
  let var2 = double(var0);
  jump -> Block 1
Block 3:
  let var3 = is_negative(var0);
  branch var3 ? Block 5 : Block 6
Block 4:
  let var6 = phi([block 5, var4], [block 6, var5]);
  jump -> Block 1
Block 5:
  let var4 = double(var0);
  jump -> Block 4
Block 6:
  let var5 = identity(var0);
  jump -> Block 4";
    assert_eq!(graph.graph_string().trim(), expected_graph.trim());
}

#[test]
fn while_loop_graph_structure() {
    #[workflow]
    fn test_workflow() -> i32 {
        let mut i = 0;
        while less_than(i, 3) {
            i = add(i, 1);
        }
        i
    }

    let graph = test_workflow();
    let expected = "Block 0:
  let var0 = 0;
  jump -> Block 1
Block 1:
  let var1 = phi([block 0, var0], [block 2, var5]);
  let var2 = 3;
  let var3 = less_than(var1, var2);
  branch var3 ? Block 2 : Block 3
Block 2:
  let var4 = 1;
  let var5 = add(var1, var4);
  jump -> Block 1
Block 3:
  return var1";
    assert_eq!(graph.graph_string().trim(), expected.trim());
}

#[test]
#[ignore]
fn conditional_in_while_loop_graph_structure() {
    #[workflow]
    fn conditional_in_while_loop() -> i32 {
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

    let graph = conditional_in_while_loop();
    let expected = "Block 0:
  let var0 = 6;
  let var1 = 0;
  jump -> Block 1
Block 1:
  let var2 = phi([block 0, var0], [block 4, var8]);
  let var3 = phi([block 0, var1], [block 4, var10]);
  let var4 = not_one(var2);
  branch var4 ? Block 2 : Block 3
Block 2:
  let var5 = is_even(var2);
  branch var5 ? Block 5 : Block 6
Block 3:
  return var3
Block 4:
  let var8 = phi([block 5, var6], [block 6, var7]);
  let var9 = 1;
  let var10 = add(var3, var9);
  jump -> Block 1
Block 5:
  let var6 = divide_by_2(var2);
  jump -> Block 4
Block 6:
  let var7 = multiply_by_3_and_add_1(var2);
  jump -> Block 4";
    assert_eq!(graph.graph_string().trim(), expected.trim());
}
