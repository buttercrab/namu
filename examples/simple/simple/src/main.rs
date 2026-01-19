use namu_engine::engine::Engine;
use namu_engine::simple_engine::SimpleEngine;
use simple::simple;

fn main() {
    let graph = simple();
    println!("{}", graph.graph_string());
    let serialized = graph.to_serializable("simple".to_string());
    let json = serde_json::to_string_pretty(&serialized).unwrap();
    println!("{}", json);

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let engine = SimpleEngine::with_registered();
        let wf_id = engine.create_workflow(serialized).await;
        let run_id = engine.create_run(wf_id).await;
        let engine_clone = engine.clone();
        let handle = tokio::spawn(async move { engine_clone.run(run_id).await });
        let rx = engine.get_result(run_id);
        let rx = rx.as_async();
        let result = rx.recv().await.unwrap();
        println!("result: {:?}", result.downcast_ref::<i32>().unwrap());
        handle.await.unwrap().unwrap();
    });
}
