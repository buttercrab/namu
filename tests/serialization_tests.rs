#![allow(dead_code)]

mod common;

use namu::workflow;
use namu_core::ir::Workflow;

use crate::common::*;

#[test]
fn serializable_conditional_workflow() {
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
    let serializable = graph.to_serializable("conditional".to_string());

    let expected = r#"{
  "name": "conditional",
  "operations": [
    {
      "literals": [ { "output": 0, "value": "10" } ],
      "phis": [],
      "action": {
        "Call": { "task_id": "is_positive", "inputs": [0], "outputs": [1] }
      },
      "next": {
        "Branch": { "var": 1, "true_next": 2, "false_next": 3 }
      }
    },
    {
      "literals": [],
      "phis": [ { "output": 4, "from": [[2, 2], [3, 3]] } ],
      "next": { "Return": { "var": 4 } }
    },
    {
      "literals": [],
      "phis": [],
      "action": {
        "Call": { "task_id": "double", "inputs": [0], "outputs": [2] }
      },
      "next": { "Jump": { "next": 1 } }
    },
    {
      "literals": [],
      "phis": [],
      "action": {
        "Call": { "task_id": "identity", "inputs": [0], "outputs": [3] }
      },
      "next": { "Jump": { "next": 1 } }
    }
  ],
  "types": [{ "int": { "bits": 32, "signed": true } }, "bool", { "int": { "bits": 32, "signed": true } }, { "int": { "bits": 32, "signed": true } }, { "int": { "bits": 32, "signed": true } }]
}"#;

    let expected: Workflow = serde_json::from_str(expected).unwrap();
    assert_eq!(serializable, expected);
}

#[test]
fn serializable_while_loop_workflow() {
    #[workflow]
    fn test_workflow() -> i32 {
        let mut i = 0;
        while less_than(i, 3) {
            i = add(i, 1);
        }
        i
    }

    let graph = test_workflow();
    let serializable = graph.to_serializable("while_loop".to_string());

    let expected = r#"{
  "name": "while_loop",
  "operations": [
    {
      "literals": [
        { "output": 0, "value": "0" },
        { "output": 2, "value": "3" },
        { "output": 4, "value": "1" }
      ],
      "phis": [],
      "next": { "Jump": { "next": 1 } }
    },
    {
      "literals": [],
      "phis": [ { "output": 1, "from": [[0,0],[2,5]] } ],
      "action": {
        "Call": { "task_id": "less_than", "inputs": [1,2], "outputs": [3] }
      },
      "next": { "Branch": { "var": 3, "true_next": 2, "false_next": 3 } }
    },
    {
      "literals": [],
      "phis": [],
      "action": {
        "Call": { "task_id": "add", "inputs": [1,4], "outputs": [5] }
      },
      "next": { "Jump": { "next": 1 } }
    },
    {
      "literals": [],
      "phis": [],
      "next": { "Return": { "var": 1 } }
    }
  ],
  "types": [{ "int": { "bits": 32, "signed": true } }, { "int": { "bits": 32, "signed": true } }, { "int": { "bits": 32, "signed": true } }, "bool", { "int": { "bits": 32, "signed": true } }, { "int": { "bits": 32, "signed": true } }]
}"#;

    let expected: Workflow = serde_json::from_str(expected).unwrap();
    assert_eq!(serializable, expected);
}
