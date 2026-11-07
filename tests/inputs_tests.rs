//! Workflow parameters: a run's inputs. They are in the root context, so every branch sees them,
//! and they are checked against the parameters' types before a run starts.

mod common;

use std::sync::Arc;

use namu::{Inputs, workflow};
use namu_core::ir::Input;
use namu_core::types::Type;
use namu_engine::{LocalCluster, Tasks};
use serde_json::json;

use crate::common::*;

#[test]
fn parameters_are_inputs_in_the_ir() {
    #[workflow]
    fn wf(n: i32, name: String) -> i32 {
        add(double(n), length(name))
    }

    let ir = wf().to_serializable("w".into());
    let names: Vec<&str> = ir.inputs.iter().map(|input| input.name.as_str()).collect();
    assert_eq!(names, ["n", "name"]);
    let [Input { output: n, .. }, Input { output: name, .. }] = &ir.inputs[..] else {
        panic!("two inputs: {:?}", ir.inputs);
    };
    assert_eq!(
        ir.types[*n],
        Type::Int {
            bits: 32,
            signed: true
        }
    );
    assert_eq!(ir.types[*name], Type::String);
}

#[tokio::test(flavor = "multi_thread")]
async fn inputs_reach_every_branch() {
    #[workflow]
    fn wf(start: i32, end: i32, offset: i32) -> i32 {
        let xs = range(start, end);
        let shifted = add(xs, offset);
        sum(shifted)
    }

    let inputs = Inputs::new()
        .with("start", 0)
        .with("end", 5)
        .with("offset", 1);
    let outcome = run_with(wf().to_serializable("w".into()), inputs).await;
    assert!(outcome.succeeded(), "failures: {:?}", outcome.failures);
    // 0, 10, 20, 30, 40, each plus 1.
    assert_eq!(outcome.results, [json!(105)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_input_is_a_variable_like_any_other() {
    #[workflow]
    fn wf(mut n: i32, limit: i32) -> i32 {
        while less_than(n, limit) {
            n = double(n);
        }
        n
    }

    let inputs = Inputs::new().with("n", 3).with("limit", 40);
    let outcome = run_with(wf().to_serializable("w".into()), inputs).await;
    assert_eq!(outcome.results, [json!(48)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn inputs_come_as_json() {
    #[workflow]
    fn wf(word: String, times: i32) -> String {
        let mut s = word;
        let mut i = 0;
        while i < times {
            s = exclaim(s);
            i += 1;
        }
        s
    }

    let object = json!({"word": "hi", "times": 3});
    let inputs = Inputs::from_json(object.as_object().unwrap().clone());
    let outcome = run_with(wf().to_serializable("w".into()), inputs).await;
    assert_eq!(outcome.results, [json!("hi!!!")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_inputs_are_refused_before_the_run_starts() {
    #[workflow]
    fn wf(n: i32, name: String) -> i32 {
        add(double(n), length(name))
    }

    let tasks: Arc<dyn Tasks> = Arc::new(tasks());
    let cluster = LocalCluster::uniform(1, tasks);
    let refused = |inputs: Inputs| {
        let cluster = &cluster;
        async move {
            cluster
                .run_with(wf().to_serializable("w".into()), inputs)
                .await
                .expect_err("refused")
                .to_string()
        }
    };

    let missing = refused(Inputs::new().with("n", 1)).await;
    assert!(missing.contains("`name` (String) is missing"), "{missing}");

    let unknown = refused(
        Inputs::new()
            .with("n", 1)
            .with("name", "x".to_string())
            .with("nme", 2),
    )
    .await;
    assert!(
        unknown.contains("`nme` is not a parameter of w"),
        "{unknown}"
    );

    let typed = refused(Inputs::new().with("n", 1i64).with("name", "x".to_string())).await;
    assert!(typed.contains("`n`: expected i32, got i64"), "{typed}");

    let object = json!({"n": "one", "name": "x"});
    let json = refused(Inputs::from_json(object.as_object().unwrap().clone())).await;
    assert!(json.contains("`n`: expected i32, got a string"), "{json}");

    let object = json!({"n": 3_000_000_000_u64, "name": "x"});
    let json = refused(Inputs::from_json(object.as_object().unwrap().clone())).await;
    assert!(json.contains("does not fit in i32"), "{json}");

    // A workflow without parameters takes no inputs.
    #[workflow]
    fn bare() -> i32 {
        double(1)
    }
    let extra = cluster
        .run_with(
            bare().to_serializable("bare".into()),
            Inputs::new().with("n", 1),
        )
        .await
        .expect_err("refused")
        .to_string();
    assert!(extra.contains("`n` is not a parameter of bare"), "{extra}");
}
