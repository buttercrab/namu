use macros::workflow;
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn simple_return_workflow() -> graph::Graph<i32> {
    let __builder = graph::Builder::<i32>::new();
    let __result = { graph::new_literal(&__builder, 123) };
    __builder.seal_block(graph::Terminator::return_value(__result.id));
    __builder.build()
}
