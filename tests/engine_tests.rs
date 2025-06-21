// new file

mod common;

use namu::workflow;
use namu_core::{DynamicTaskContext, Task, Value};
use namu_engine::context::dynamic_context::DynamicContextManager;
use namu_engine::engine::simple_engine::SimpleEngine;
use namu_engine::engine::{Engine, PackFn};

use crate::common::*;

// ---- Hand-written task implementation ------------------------------------

#[derive(Clone)]
struct AddTask;

impl<Id> namu_core::SingleTask<Id, DynamicTaskContext<Id>> for AddTask
where
    Id: Clone + Send,
{
    type Input = (i32, i32);
    type Output = i32;

    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        Ok(input.0 + input.1)
    }
}

impl<Id> Task<Id, DynamicTaskContext<Id>> for AddTask
where
    Id: Clone + Send,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn clone_box(&self) -> Box<dyn Task<Id, DynamicTaskContext<Id>> + Send + Sync> {
        Box::new(self.clone())
    }

    fn run(&mut self, ctx: DynamicTaskContext<Id>) -> anyhow::Result<()> {
        <Self as namu_core::SingleTask<Id, DynamicTaskContext<Id>>>::run(self, ctx)
    }
}

// ---- Additional task: LessThanTask --------------------------------------

#[derive(Clone)]
struct LessThanTask;

impl<Id> namu_core::SingleTask<Id, DynamicTaskContext<Id>> for LessThanTask
where
    Id: Clone + Send,
{
    type Input = (i32, i32);
    type Output = bool;

    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        Ok(input.0 < input.1)
    }
}

impl<Id> Task<Id, DynamicTaskContext<Id>> for LessThanTask
where
    Id: Clone + Send,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn clone_box(&self) -> Box<dyn Task<Id, DynamicTaskContext<Id>> + Send + Sync> {
        Box::new(self.clone())
    }

    fn run(&mut self, ctx: DynamicTaskContext<Id>) -> anyhow::Result<()> {
        <Self as namu_core::SingleTask<Id, DynamicTaskContext<Id>>>::run(self, ctx)
    }
}

// ---- Helper pack / unpack -------------------------------------------------

fn pack_add(inputs: Vec<Value>) -> Value {
    let a = *inputs[0].downcast_ref::<i32>().unwrap();
    let b = *inputs[1].downcast_ref::<i32>().unwrap();
    Value::new((a, b))
}

// ---- Helper pack / unpack for less_than ---------------------------------

fn pack_less_than(inputs: Vec<Value>) -> Value {
    let a = *inputs[0].downcast_ref::<i32>().unwrap();
    let b = *inputs[1].downcast_ref::<i32>().unwrap();
    Value::new((a, b))
}

// ---- The actual test ------------------------------------------------------

#[test]
fn engine_executes_simple_workflow() {
    // 1. Build workflow IR.
    #[workflow]
    fn simple_workflow() -> i32 {
        let a = 1;
        let b = 2;
        add(a, b)
    }

    let graph = simple_workflow();
    let wf_ir = graph.to_serializable("simple".to_string());

    // 2. Create engine.
    let engine = SimpleEngine::new(DynamicContextManager::new());

    // 3. Register task.
    engine.add_task("add", Box::new(AddTask), Some(pack_add as PackFn), None);

    // 4. Run workflow.
    let wf_id = engine.create_workflow(wf_ir);
    let run_id = engine.create_run(wf_id);
    engine.run(run_id);

    // 5. Await and assert result.
    let result = engine.get_result(run_id);

    let val = *result.recv().unwrap().downcast_ref::<i32>().unwrap();
    assert_eq!(val, 3);
}

// ---- Fibonacci workflow test --------------------------------------------

#[test]
fn engine_executes_fibonacci_workflow() {
    // 1. Build workflow IR.
    #[workflow]
    fn fibonacci_workflow() -> i32 {
        let mut a = 0;
        let mut b = 1;

        while less_than(a, 10) {
            let c = add(a, b);
            a = b;
            b = c;
        }

        b
    }

    let graph = fibonacci_workflow();
    let wf_ir = graph.to_serializable("fibonacci".to_string());

    // 2. Create engine.
    let engine = SimpleEngine::new(DynamicContextManager::new());

    // 3. Register tasks.
    engine.add_task("add", Box::new(AddTask), Some(pack_add as PackFn), None);
    engine.add_task(
        "less_than",
        Box::new(LessThanTask),
        Some(pack_less_than as PackFn),
        None,
    );

    // 4. Run workflow.
    let wf_id = engine.create_workflow(wf_ir);
    let run_id = engine.create_run(wf_id);
    engine.run(run_id);

    // 5. Await and assert result.
    let result = engine.get_result(run_id);

    let val = *result.recv().unwrap().downcast_ref::<i32>().unwrap();
    assert_eq!(val, 21);
}
