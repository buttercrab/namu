use namu_macros::{task, workflow};
fn do_nothing<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "do_nothing", __inputs)
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
#[allow(unused_mut)]
pub fn if_statement_workflow() -> ::namu::__macro_exports::Graph<()> {
    let __builder = ::namu::__macro_exports::Builder::<()>::new();
    {
        let x = ::namu::__macro_exports::literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_condition = ::namu::__macro_exports::compare(
                &__builder,
                ::namu::__macro_exports::BuiltinOp::Gt,
                x,
                ::namu::__macro_exports::literal(&__builder, 5),
            );
            let __if_parent_predecessor_0 = __builder.current_block_id();
            ::namu::__macro_exports::branch(
                &__builder,
                __if_condition,
                __if_then_block_0,
                __if_merge_block_0,
            );
            __builder.switch_to_block(__if_then_block_0);
            let __then_val = {
                do_nothing(&__builder, x);
            };
            let __then_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __if_merge_block_0);
            __builder.switch_to_block(__if_merge_block_0);
        }
    };
    ::namu::__macro_exports::return_unit(&__builder);
    __builder.build()
}
fn __namu_build_if_statement_workflow() -> ::namu::__macro_exports::Workflow {
    if_statement_workflow().to_serializable("if_statement_workflow".to_string())
}
