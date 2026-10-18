use namu_macros::{task, workflow};
fn double<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "double", __inputs)
}
fn identity<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "identity", __inputs)
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn if_else_return_value_workflow() -> ::namu::__macro_exports::Graph<i32> {
    let __builder = ::namu::__macro_exports::Builder::<i32>::new();
    let __result = {
        let x = ::namu::__macro_exports::literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_else_block_0 = __builder.new_block();
            let __if_condition = x > ::namu::__macro_exports::literal(&__builder, 5);
            let __if_parent_predecessor_0 = __builder.current_block_id();
            ::namu::__macro_exports::branch(
                &__builder,
                __if_condition,
                __if_then_block_0,
                __if_else_block_0,
            );
            __builder.switch_to_block(__if_then_block_0);
            let __then_val = { double(&__builder, x) };
            let __then_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __if_merge_block_0);
            __builder.switch_to_block(__if_else_block_0);
            let __else_val = { { identity(&__builder, x) } };
            let __else_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __if_merge_block_0);
            __builder.switch_to_block(__if_merge_block_0);
            ::namu::__macro_exports::phi(
                &__builder,
                ::alloc::boxed::box_assume_init_into_vec_unsafe(
                    ::alloc::intrinsics::write_box_via_move(
                        ::alloc::boxed::Box::new_uninit(),
                        [
                            (__then_predecessor_id_0, __then_val),
                            (__else_predecessor_id_0, __else_val),
                        ],
                    ),
                ),
            )
        }
    };
    ::namu::__macro_exports::return_value(&__builder, __result);
    __builder.build()
}
fn __namu_build_if_else_return_value_workflow() -> ::namu::__macro_exports::Workflow {
    if_else_return_value_workflow()
        .to_serializable("if_else_return_value_workflow".to_string())
}
