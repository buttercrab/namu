use std::thread;

use namu_engine::{
    context::dynamic_context::DynamicContextManager,
    engine::{Engine, simple_engine::SimpleEngine},
};
use simple::simple;

fn main() {
    let graph = simple();
    println!("{}", graph.graph_string());
    let serialized = graph.to_serializable("simple".to_string());
    let json = serde_json::to_string_pretty(&serialized).unwrap();
    println!("{}", json);

    let engine = SimpleEngine::with_registered(DynamicContextManager::new());
    let wf_id = engine.create_workflow(serialized);
    let run_id = engine.create_run(wf_id);
    let engine_clone = engine.clone();
    let handle = thread::spawn(move || engine_clone.run(run_id));
    let result = engine.get_result(run_id);
    println!(
        "result: {:?}",
        result.recv().unwrap().downcast_ref::<i32>().unwrap()
    );
    handle.join().unwrap();
}
