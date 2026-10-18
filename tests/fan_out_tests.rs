#![allow(dead_code)]

use namu::prelude::*;

pub struct Stats;

// Signatures only: these tasks are implemented elsewhere (e.g. as WASM components).
#[task]
pub fn split_docs(n: i32) -> Result<Vec<String>>;

#[task]
pub fn analyze(doc: String, min_len: i32) -> Result<Stats>;

#[task]
pub fn summarize(stats: Vec<Stats>) -> Result<i32>;

#[task]
pub fn shout(s: String) -> Result<String>;

#[task]
pub fn is_long(s: String) -> Result<bool>;

#[test]
fn passing_a_list_fans_out_and_passing_items_collects() {
    #[workflow]
    fn wf() -> i32 {
        let n = 3;
        let min_len = 2;
        let docs = split_docs(n);
        let stats = analyze(docs, min_len); // Vec<String> -> String: fan-out
        summarize(stats) // Stats -> Vec<Stats>: collect
    }

    let expected = "Block 0:
  let var0 = 3;
  let var1 = 2;
  let var2 = split_docs(var0);
  let var3 = fan_out(var2);
  let var4 = analyze(var3, var1);
  let var5 = collect(var4);
  let var6 = summarize(var5);
  return var6";
    assert_eq!(wf().graph_string().trim(), expected);

    let ir = wf().to_serializable("wf".to_string());
    let fan_out = ir.operations.iter().find_map(|op| op.fan_out.as_ref());
    let collect = ir.operations.iter().find_map(|op| op.collect.as_ref());
    assert_eq!(fan_out.map(|f| (f.input, f.output)), Some((2, 3)));
    assert_eq!(collect.map(|c| (c.input, c.output)), Some((4, 5)));
    // Each action (call, fan-out, collect) gets its own operation.
    assert!(ir.operations.iter().all(|op| {
        [
            op.call.is_some(),
            op.fan_out.is_some(),
            op.collect.is_some(),
        ]
        .iter()
        .filter(|set| **set)
        .count()
            <= 1
    }));
}

#[test]
fn non_copy_mut_values_work_across_control_flow() {
    #[workflow]
    fn wf() -> String {
        let mut s = "hi";
        if is_long(s) {
            s = shout(s);
        }
        s
    }

    let ir = wf().to_serializable("wf".to_string());
    assert!(
        ir.operations
            .iter()
            .any(|op| op.call.as_ref().is_some_and(|c| c.task_id == "shout"))
    );
}

#[test]
fn literals_are_json_encoded() {
    #[workflow]
    fn wf() -> String {
        let s = "say \"hi\"";
        shout(s)
    }

    let ir = wf().to_serializable("wf".to_string());
    assert_eq!(ir.operations[0].literals[0].value, r#""say \"hi\"""#);
}
