use graph::{task, workflow};

// --- Test Tasks ---

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

#[task]
fn is_positive(v: i32) -> bool {
    v > 0
}

#[task]
fn double(v: i32) -> i32 {
    v * 2
}

#[task]
fn identity(v: i32) -> i32 {
    v
}

#[task]
fn is_negative(v: i32) -> bool {
    v < 0
}

#[task]
fn panicker() -> i32 {
    panic!("This should not be called!");
}

// --- Run Tests ---

#[test]
fn simple_workflow_run() {
    #[workflow]
    fn test_workflow() -> i32 {
        let a = one();
        let b = two();
        add(a, b)
    }

    let graph = test_workflow();
    assert_eq!(graph.run(), 3);
}

#[test]
fn reassignment_workflow_run() {
    #[workflow]
    fn test_workflow() -> i32 {
        let a = one();
        let mut b = two();
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
        val = if is_positive(val) { double(val) } else { val };
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
        let mut b = input;
        b = if is_positive(input) {
            double(input)
        } else {
            identity(input)
        };
        add(b, b)
    }

    let graph = test_workflow();
    assert_eq!(graph.run(), 40);
}

// --- Graph Structure Tests ---

#[test]
fn simple_graph_structure() {
    #[workflow]
    fn test_workflow() -> i32 {
        let a = one();
        add(a, a)
    }

    let graph = test_workflow();
    let expected_graph_str = "Block 0:
  let var0 = one();
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

// --- Compile-Fail Tests ---

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile-fail/*.rs");
}
