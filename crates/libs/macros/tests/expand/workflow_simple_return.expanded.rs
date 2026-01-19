use namu_macros::workflow;
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn simple_return_workflow() -> ::namu::__macro_exports::Graph<i32> {
    let __builder = ::namu::__macro_exports::Builder::<i32>::new();
    let __result = { ::namu::__macro_exports::literal(&__builder, 123) };
    ::namu::__macro_exports::return_value(&__builder, __result);
    __builder.build()
}
fn __namu_build_simple_return_workflow() -> ::namu::__macro_exports::Workflow {
    simple_return_workflow().to_serializable("simple_return_workflow".to_string())
}
