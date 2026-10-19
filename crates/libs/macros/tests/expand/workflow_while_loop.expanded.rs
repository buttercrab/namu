use namu_macros::{task, workflow};
fn less_than<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
    b: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<bool> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
        ::namu::__macro_exports::IntoArg::into_arg(b, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "less_than", __inputs)
}
fn add_one<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "add_one", __inputs)
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
#[allow(unused_mut)]
pub fn while_loop_workflow() -> ::namu::__macro_exports::Graph<i32> {
    let __builder = ::namu::__macro_exports::Builder::<i32>::new();
    let __result = {
        let mut i = ::namu::__macro_exports::literal(&__builder, 0);
        {
            let __pre_while_i_0 = i;
            let mut __while_breaks_0: ::std::vec::Vec<(usize, ::std::vec::Vec<usize>)> = ::std::vec::Vec::new();
            let mut __while_continues_0: ::std::vec::Vec<
                (usize, ::std::vec::Vec<usize>),
            > = ::std::vec::Vec::new();
            let __while_header_block_0 = __builder.new_block();
            let __while_body_block_0 = __builder.new_block();
            let __while_exit_block_0 = __builder.new_block();
            let __while_parent_predecessor_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __while_header_block_0);
            __builder.switch_to_block(__while_header_block_0);
            let __i_phi_val_0 = {
                let __phi_id = __builder
                    .phi(
                        ::alloc::boxed::box_assume_init_into_vec_unsafe(
                            ::alloc::intrinsics::write_box_via_move(
                                ::alloc::boxed::Box::new_uninit(),
                                [(__while_parent_predecessor_0, __pre_while_i_0.id)],
                            ),
                        ),
                    );
                ::namu::__macro_exports::TracedValue::new(__phi_id)
            };
            i = __i_phi_val_0;
            let __i_phi_node_id_0 = __builder.arena().nodes.len() - 1;
            let __while_cond = less_than(
                &__builder,
                i,
                ::namu::__macro_exports::literal(&__builder, 5),
            );
            let __while_header_exit_0 = __builder.current_block_id();
            ::namu::__macro_exports::branch(
                &__builder,
                __while_cond,
                __while_body_block_0,
                __while_exit_block_0,
            );
            __builder.switch_to_block(__while_body_block_0);
            {
                i = add_one(&__builder, i);
            };
            let __post_body_i_0 = i;
            let __body_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __while_header_block_0);
            if let Some(
                ::namu::__macro_exports::Node {
                    kind: ::namu::__macro_exports::NodeKind::Phi { from },
                    ..
                },
            ) = __builder.arena_mut().nodes.get_mut(__i_phi_node_id_0)
            {
                *from = ::alloc::boxed::box_assume_init_into_vec_unsafe(
                    ::alloc::intrinsics::write_box_via_move(
                        ::alloc::boxed::Box::new_uninit(),
                        [
                            (__while_parent_predecessor_0, __pre_while_i_0.id),
                            (__body_predecessor_id_0, __post_body_i_0.id),
                        ],
                    ),
                );
                from.extend(
                    __while_continues_0.iter().map(|(block, ids)| (*block, ids[0usize])),
                );
            }
            __builder.switch_to_block(__while_exit_block_0);
            i = if __while_breaks_0.is_empty() {
                __i_phi_val_0
            } else {
                let __from = ::std::iter::once((__while_header_exit_0, __i_phi_val_0.id))
                    .chain(
                        __while_breaks_0.iter().map(|(block, ids)| (*block, ids[0usize])),
                    )
                    .collect();
                ::namu::__macro_exports::TracedValue::new(__builder.phi(__from))
            };
        }
        i
    };
    ::namu::__macro_exports::return_value(&__builder, __result);
    __builder.build()
}
fn __namu_build_while_loop_workflow() -> ::namu::__macro_exports::Workflow {
    while_loop_workflow().to_serializable("while_loop_workflow".to_string())
}
