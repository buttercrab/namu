#![allow(dead_code)]

mod common;

use crate::common::*;
use namu::workflow;
use namu_core::ir::Workflow;
use serde_json;

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
      "call": {
        "task_id": "is_positive",
        "inputs": [0],
        "outputs": [1]
      },
      "next": {
        "Branch": { "var": 1, "true_next": 2, "false_next": 3 }
      }
    },
    {
      "literals": [],
      "phis": [ { "output": 4, "from": [[2, 2], [3, 3]] } ],
      "call": null,
      "next": { "Return": { "var": 4 } }
    },
    {
      "literals": [],
      "phis": [],
      "call": {
        "task_id": "double",
        "inputs": [0],
        "outputs": [2]
      },
      "next": { "Jump": { "next": 1 } }
    },
    {
      "literals": [],
      "phis": [],
      "call": {
        "task_id": "identity",
        "inputs": [0],
        "outputs": [3]
      },
      "next": { "Jump": { "next": 1 } }
    }
  ]
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
      "literals": [ { "output": 0, "value": "0" } ],
      "phis": [],
      "call": null,
      "next": { "Jump": { "next": 1 } }
    },
    {
      "literals": [ { "output": 2, "value": "3" } ],
      "phis": [ { "output": 1, "from": [[0,0],[2,5]] } ],
      "call": {
        "task_id": "less_than",
        "inputs": [1,2],
        "outputs": [3]
      },
      "next": { "Branch": { "var": 3, "true_next": 2, "false_next": 3 } }
    },
    {
      "literals": [ { "output": 4, "value": "1" } ],
      "phis": [],
      "call": {
        "task_id": "add",
        "inputs": [1,4],
        "outputs": [5]
      },
      "next": { "Jump": { "next": 1 } }
    },
    {
      "literals": [ { "output": 6, "value": "()" } ],
      "phis": [],
      "call": null,
      "next": { "Return": { "var": 1 } }
    }
  ]
}"#;

    let expected: Workflow = serde_json::from_str(expected).unwrap();
    assert_eq!(serializable, expected);
}
