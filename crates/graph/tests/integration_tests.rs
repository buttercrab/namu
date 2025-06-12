use anyhow::Result;
use graph::{task, workflow};

// --- Test Tasks ---

#[task]
fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}

#[task]
fn is_positive(v: i32) -> Result<bool> {
    Ok(v > 0)
}

#[task]
fn double(v: i32) -> Result<i32> {
    Ok(v * 2)
}

#[task]
fn identity(v: i32) -> Result<i32> {
    Ok(v)
}

#[task]
fn is_negative(v: i32) -> Result<bool> {
    Ok(v < 0)
}

#[task]
fn less_than(a: i32, b: i32) -> Result<bool> {
    Ok(a < b)
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

#[task]
fn not_one(n: i32) -> Result<bool> {
    Ok(n != 1)
}

#[task]
fn panicker() -> Result<i32> {
    panic!("This should not be called!");
}

// --- Run Tests ---

#[test]
fn simple_workflow_run() {
    #[workflow]
    fn test_workflow() -> i32 {
        let a = 1;
        let b = 2;
        add(a, b)
    }

    let graph = test_workflow();
    assert_eq!(graph.run(), 3);
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
    assert_eq!(graph.run(), 4);
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
    assert_eq!(graph.run(), 20);
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
    assert_eq!(graph.run(), 20);
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
    assert_eq!(graph.run(), 20);
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
    assert_eq!(graph.run(), 20);
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
    assert_eq!(graph.run(), 40);
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
    assert_eq!(graph.run(), 10);
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
    assert_eq!(graph.run(), 15);
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
    assert_eq!(graph.run(), 8);
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
    assert_eq!(graph.run(), 10);
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
    assert_eq!(graph.run(), 13);
}

// --- Graph Structure Tests ---

#[test]
fn simple_graph_structure() {
    #[workflow]
    fn test_workflow() -> i32 {
        let a = 1;
        add(a, a)
    }

    let graph = test_workflow();
    let expected_graph_str = "Block 0:
  let var0 = 1;
  let var1 = add(var0, var0);
  return var1
";
    assert_eq!(graph.graph_string().trim(), expected_graph_str.trim());
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
    let expected_graph_str = "Block 0:
  let var0 = 10;
  let var1 = is_positive(var0);
  branch var1 ? Block 1 : Block 2
Block 1:
  let var2 = double(var0);
  jump -> Block 3
Block 2:
  let var3 = identity(var0);
  jump -> Block 3
Block 3:
  let var4 = phi([block 1, var2], [block 2, var3]);
  return var4
";
    assert_eq!(graph.graph_string().trim(), expected_graph_str.trim());
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
    let expected_graph_str = "Block 0:
  let var0 = 10;
  let var1 = is_positive(var0);
  branch var1 ? Block 1 : Block 2
Block 1:
  let var2 = double(var0);
  jump -> Block 3
Block 2:
  let var3 = is_negative(var0);
  branch var3 ? Block 4 : Block 5
Block 3:
  let var7 = phi([block 1, var2], [block 6, var6]);
  return var7
Block 4:
  let var4 = double(var0);
  jump -> Block 6
Block 5:
  let var5 = identity(var0);
  jump -> Block 6
Block 6:
  let var6 = phi([block 4, var4], [block 5, var5]);
  jump -> Block 3
";
    assert_eq!(graph.graph_string().trim(), expected_graph_str.trim());
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
    let expected_graph_str = r#"
Block 0:
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
  return var1
"#;
    assert_eq!(graph.graph_string().trim(), expected_graph_str.trim());
}

#[test]
fn conditional_in_while_loop_graph_structure() {
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
    let expected_graph_str = r#"
Block 0:
  let var0 = 6;
  let var1 = 0;
  jump -> Block 1
Block 1:
  let var2 = phi([block 0, var0], [block 6, var8]);
  let var3 = phi([block 0, var1], [block 6, var10]);
  let var4 = not_one(var2);
  branch var4 ? Block 2 : Block 3
Block 2:
  let var5 = is_even(var2);
  branch var5 ? Block 4 : Block 5
Block 3:
  return var3
Block 4:
  let var6 = divide_by_2(var2);
  jump -> Block 6
Block 5:
  let var7 = multiply_by_3_and_add_1(var2);
  jump -> Block 6
Block 6:
  let var8 = phi([block 4, var6], [block 5, var7]);
  let var9 = 1;
  let var10 = add(var3, var9);
  jump -> Block 1
"#;
    assert_eq!(graph.graph_string().trim(), expected_graph_str.trim());
}

// --- Compile-Fail Tests ---

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile-fail/*.rs");
}
