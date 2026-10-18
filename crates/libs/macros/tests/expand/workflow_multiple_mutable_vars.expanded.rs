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
fn add<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
    b: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
        ::namu::__macro_exports::IntoArg::into_arg(b, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "add", __inputs)
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn multiple_mutable_vars_workflow() -> ::namu::__macro_exports::Graph<i32> {
    let __builder = ::namu::__macro_exports::Builder::<i32>::new();
    let __result = {
        let mut a = ::namu::__macro_exports::literal(&__builder, 0);
        let mut b = ::namu::__macro_exports::literal(&__builder, 1);
        let mut i = ::namu::__macro_exports::literal(&__builder, 0);
        {
            let __pre_while_a_0 = a;
            let __pre_while_b_0 = b;
            let __pre_while_i_0 = i;
            let __while_header_block_0 = __builder.new_block();
            let __while_body_block_0 = __builder.new_block();
            let __while_exit_block_0 = __builder.new_block();
            let __while_parent_predecessor_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __while_header_block_0);
            __builder.switch_to_block(__while_header_block_0);
            let __a_phi_val_0 = {
                let __phi_id = __builder
                    .phi(
                        ::alloc::boxed::box_assume_init_into_vec_unsafe(
                            ::alloc::intrinsics::write_box_via_move(
                                ::alloc::boxed::Box::new_uninit(),
                                [(__while_parent_predecessor_0, __pre_while_a_0.id)],
                            ),
                        ),
                    );
                ::namu::__macro_exports::TracedValue::new(__phi_id)
            };
            a = __a_phi_val_0;
            let __a_phi_node_id_0 = __builder.arena().nodes.len() - 1;
            let __b_phi_val_0 = {
                let __phi_id = __builder
                    .phi(
                        ::alloc::boxed::box_assume_init_into_vec_unsafe(
                            ::alloc::intrinsics::write_box_via_move(
                                ::alloc::boxed::Box::new_uninit(),
                                [(__while_parent_predecessor_0, __pre_while_b_0.id)],
                            ),
                        ),
                    );
                ::namu::__macro_exports::TracedValue::new(__phi_id)
            };
            b = __b_phi_val_0;
            let __b_phi_node_id_0 = __builder.arena().nodes.len() - 1;
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
            ::namu::__macro_exports::branch(
                &__builder,
                __while_cond,
                __while_body_block_0,
                __while_exit_block_0,
            );
            __builder.switch_to_block(__while_body_block_0);
            {
                let temp = a;
                a = b;
                b = add(&__builder, temp, b);
                i = add(&__builder, i, ::namu::__macro_exports::literal(&__builder, 1));
            };
            let __post_body_a_0 = a;
            let __post_body_b_0 = b;
            let __post_body_i_0 = i;
            let __body_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __while_header_block_0);
            if let Some(
                ::namu::__macro_exports::Node {
                    kind: ::namu::__macro_exports::NodeKind::Phi { from },
                    ..
                },
            ) = __builder.arena_mut().nodes.get_mut(__a_phi_node_id_0)
            {
                *from = ::alloc::boxed::box_assume_init_into_vec_unsafe(
                    ::alloc::intrinsics::write_box_via_move(
                        ::alloc::boxed::Box::new_uninit(),
                        [
                            (__while_parent_predecessor_0, __pre_while_a_0.id),
                            (__body_predecessor_id_0, __post_body_a_0.id),
                        ],
                    ),
                );
            }
            if let Some(
                ::namu::__macro_exports::Node {
                    kind: ::namu::__macro_exports::NodeKind::Phi { from },
                    ..
                },
            ) = __builder.arena_mut().nodes.get_mut(__b_phi_node_id_0)
            {
                *from = ::alloc::boxed::box_assume_init_into_vec_unsafe(
                    ::alloc::intrinsics::write_box_via_move(
                        ::alloc::boxed::Box::new_uninit(),
                        [
                            (__while_parent_predecessor_0, __pre_while_b_0.id),
                            (__body_predecessor_id_0, __post_body_b_0.id),
                        ],
                    ),
                );
            }
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
            }
            __builder.switch_to_block(__while_exit_block_0);
            a = ::namu::__macro_exports::TracedValue::new(__a_phi_node_id_0);
            b = ::namu::__macro_exports::TracedValue::new(__b_phi_node_id_0);
            i = ::namu::__macro_exports::TracedValue::new(__i_phi_node_id_0);
        }
        a
    };
    ::namu::__macro_exports::return_value(&__builder, __result);
    __builder.build()
}
fn __namu_build_multiple_mutable_vars_workflow() -> ::namu::__macro_exports::Workflow {
    multiple_mutable_vars_workflow()
        .to_serializable("multiple_mutable_vars_workflow".to_string())
}
