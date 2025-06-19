use simple::simple;

fn main() {
    let graph = simple();
    println!("{}", graph.graph_string());
    let serialized = graph.to_serializable("simple".to_string());
    let json = serde_json::to_string_pretty(&serialized).unwrap();
    println!("{}", json);
}
