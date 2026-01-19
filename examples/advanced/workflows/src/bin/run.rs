use namu_core::Value;
use namu_core::ir::Workflow;
use namu_engine::simple_engine::SimpleEngine;
use namu_engine::traits::engine::Engine;

fn execute(workflow: Workflow) -> Vec<Value> {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let engine = SimpleEngine::with_registered();
        let wf_id = engine.create_workflow(workflow).await;
        let run_id = engine.create_run(wf_id).await;

        let engine_clone = engine.clone();
        let handle = tokio::spawn(async move { engine_clone.run(run_id).await });

        let rx = engine.get_result(run_id);
        let rx = rx.as_async();
        let mut values = Vec::new();
        while let Ok(value) = rx.recv().await {
            values.push(value);
        }

        handle
            .await
            .expect("engine task panicked")
            .expect("engine run failed");

        values
    })
}

fn main() {
    let workflow_name = std::env::args().nth(1).unwrap_or_else(|| "etl".to_string());

    let (id, graph) = match workflow_name.as_str() {
        "etl" => ("etl_pipeline", namu_advanced_workflows::etl_pipeline()),
        "ml" => ("ml_pipeline", namu_advanced_workflows::ml_pipeline()),
        "media" => ("media_pipeline", namu_advanced_workflows::media_pipeline()),
        other => {
            eprintln!("Unknown workflow '{other}'. Use: etl | ml | media");
            std::process::exit(1);
        }
    };

    let wf_ir = graph.to_serializable(id.to_string());
    let values = execute(wf_ir);

    println!("Results for {id}:");
    for (idx, value) in values.iter().enumerate() {
        if let Some(v) = value.downcast_ref::<i32>() {
            println!("  [{idx}] {v}");
        } else {
            println!("  [{idx}] <unknown value>");
        }
    }
}
