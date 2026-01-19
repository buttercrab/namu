// New test for direct task implementation
use namu::prelude::*;
use namu_core::{Task, TaskContext};
use namu_engine::engine::Engine;
use namu_engine::simple_engine::SimpleEngine;
use namu_macros::task_bridge;

#[task]
#[derive(Default, Clone)]
struct DoubleTask;

impl<C: TaskContext> Task<C> for DoubleTask {
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn clone_boxed(&self) -> Box<dyn Task<C> + Send + Sync> {
        Box::new(self.clone())
    }

    fn run(&mut self, ctx: C) -> anyhow::Result<()> {
        while let Ok((id, x)) = ctx.recv::<i32>() {
            let _ = ctx.send(id, Ok(x * 2));
            let _ = ctx.send_end(id);
        }
        Ok(())
    }
}

task_bridge! {
    fn_name = double_direct,
    task    = DoubleTask,
    inputs  = "i32",
    output  = "i32",
    name    = "double_direct",
    author  = "namu",
    version = "0.1",
}

#[workflow]
fn wf() -> i32 {
    let x = 4;
    double_direct(x)
}

#[tokio::test]
async fn direct_task_workflow_builds() {
    let graph = wf().to_serializable("workflow".to_string());
    let engine = SimpleEngine::with_registered();
    let workflow_id = engine.create_workflow(graph).await;
    let run_id = engine.create_run(workflow_id).await;
    let engine_clone = engine.clone();
    let handle = tokio::spawn(async move { engine_clone.run(run_id).await });
    let rx = engine.get_result(run_id);
    let rx = rx.as_async();
    let result = *rx.recv().await.unwrap().downcast_ref::<i32>().unwrap();
    handle.await.unwrap().unwrap();
    assert_eq!(result, 8);
}
