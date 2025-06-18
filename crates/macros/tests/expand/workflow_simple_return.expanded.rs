use namu_macros::workflow;
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn simple_return_workflow() -> ::namu::__macro_exports::Graph<i32> {
    let __builder = ::namu::__macro_exports::Builder::<i32>::new();
    let __result = { ::namu::__macro_exports::literal(&__builder, 123) };
    ::namu::__macro_exports::seal_block_return_value(&__builder, __result);
    __builder.build()
}
