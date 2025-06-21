use namu_core::Value;
use namu_engine::{
    context::dynamic_context::DynamicContextManager,
    engine::{Engine, simple_engine::SimpleEngine},
};
use simple::simple;

fn pack_two(inputs: Vec<Value>) -> Value {
    let a = *inputs[0].downcast_ref::<i32>().unwrap();
    let b = *inputs[1].downcast_ref::<i32>().unwrap();
    Value::new((a, b))
}

fn main() {
    let graph = simple();
    println!("{}", graph.graph_string());
    let serialized = graph.to_serializable("simple".to_string());
    let json = serde_json::to_string_pretty(&serialized).unwrap();
    println!("{}", json);

    let engine = SimpleEngine::new(DynamicContextManager::new());
    let wf_id = engine.create_workflow(serialized);
    engine.add_task("add", Box::new(add::__add), Some(pack_two), None);
    engine.add_task(
        "is_less",
        Box::new(is_less::__is_less),
        Some(pack_two),
        None,
    );
    let run_id = engine.create_run(wf_id);
    engine.run(run_id);
    let result = engine.get_result(run_id);
    println!("result: {:?}", result.recv().unwrap());
}
