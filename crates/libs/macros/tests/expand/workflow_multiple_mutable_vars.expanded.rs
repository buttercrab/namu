use namu_macros::{task, workflow};
#[allow(non_snake_case)]
pub mod less_than {
    use super::*;
    fn task_impl(a: i32, b: i32) -> anyhow::Result<bool> {
        Ok(a < b)
    }
    #[allow(non_camel_case_types)]
    pub struct Task;
    #[automatically_derived]
    #[allow(non_camel_case_types)]
    impl ::core::clone::Clone for Task {
        #[inline]
        fn clone(&self) -> Task {
            *self
        }
    }
    #[automatically_derived]
    #[allow(non_camel_case_types)]
    impl ::core::marker::Copy for Task {}
    impl<C> ::namu::__macro_exports::Task<C> for Task
    where
        C: ::namu::__macro_exports::TaskContext,
    {
        fn prepare(&mut self) -> ::namu::__macro_exports::Result<()> {
            Ok(())
        }
        fn clone_boxed(
            &self,
        ) -> Box<dyn ::namu::__macro_exports::Task<C> + Send + Sync> {
            Box::new(*self)
        }
        fn run(&mut self, context: C) -> ::namu::__macro_exports::Result<()> {
            ::namu::__macro_exports::SingleTask::run(self, context)
        }
    }
    impl<C> ::namu::__macro_exports::SingleTask<C> for Task
    where
        C: ::namu::__macro_exports::TaskContext,
    {
        type Input = (i32, i32);
        type Output = bool;
        fn call(
            &mut self,
            input: Self::Input,
        ) -> ::namu::__macro_exports::Result<Self::Output> {
            let (a, b) = input;
            task_impl(a, b)
        }
    }
    #[allow(dead_code)]
    pub fn pack(
        mut inputs: Vec<::namu::__macro_exports::Value>,
    ) -> ::namu::__macro_exports::Value {
        if true {
            match (&inputs.len(), &2usize) {
                (left_val, right_val) => {
                    if !(*left_val == *right_val) {
                        let kind = ::core::panicking::AssertKind::Eq;
                        ::core::panicking::assert_failed(
                            kind,
                            &*left_val,
                            &*right_val,
                            ::core::option::Option::None,
                        );
                    }
                }
            };
        }
        let v0 = {
            let val = inputs.remove(0);
            (*val.downcast_ref::<i32>().expect("pack downcast failed")).clone()
        };
        let v1 = {
            let val = inputs.remove(0);
            (*val.downcast_ref::<i32>().expect("pack downcast failed")).clone()
        };
        ::namu::__macro_exports::Value::new((v0, v1))
    }
    #[allow(dead_code)]
    pub fn unpack(
        val: ::namu::__macro_exports::Value,
    ) -> Vec<::namu::__macro_exports::Value> {
        <[_]>::into_vec(::alloc::boxed::box_new([val]))
    }
}
#[allow(non_snake_case)]
pub fn less_than<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
    b: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<bool> {
    ::namu::__macro_exports::call(
        &builder,
        "less_than",
        <[_]>::into_vec(::alloc::boxed::box_new([a.id, b.id])),
    )
}
#[allow(non_snake_case)]
pub mod add {
    use super::*;
    fn task_impl(a: i32, b: i32) -> anyhow::Result<i32> {
        Ok(a + b)
    }
    #[allow(non_camel_case_types)]
    pub struct Task;
    #[automatically_derived]
    #[allow(non_camel_case_types)]
    impl ::core::clone::Clone for Task {
        #[inline]
        fn clone(&self) -> Task {
            *self
        }
    }
    #[automatically_derived]
    #[allow(non_camel_case_types)]
    impl ::core::marker::Copy for Task {}
    impl<C> ::namu::__macro_exports::Task<C> for Task
    where
        C: ::namu::__macro_exports::TaskContext,
    {
        fn prepare(&mut self) -> ::namu::__macro_exports::Result<()> {
            Ok(())
        }
        fn clone_boxed(
            &self,
        ) -> Box<dyn ::namu::__macro_exports::Task<C> + Send + Sync> {
            Box::new(*self)
        }
        fn run(&mut self, context: C) -> ::namu::__macro_exports::Result<()> {
            ::namu::__macro_exports::SingleTask::run(self, context)
        }
    }
    impl<C> ::namu::__macro_exports::SingleTask<C> for Task
    where
        C: ::namu::__macro_exports::TaskContext,
    {
        type Input = (i32, i32);
        type Output = i32;
        fn call(
            &mut self,
            input: Self::Input,
        ) -> ::namu::__macro_exports::Result<Self::Output> {
            let (a, b) = input;
            task_impl(a, b)
        }
    }
    #[allow(dead_code)]
    pub fn pack(
        mut inputs: Vec<::namu::__macro_exports::Value>,
    ) -> ::namu::__macro_exports::Value {
        if true {
            match (&inputs.len(), &2usize) {
                (left_val, right_val) => {
                    if !(*left_val == *right_val) {
                        let kind = ::core::panicking::AssertKind::Eq;
                        ::core::panicking::assert_failed(
                            kind,
                            &*left_val,
                            &*right_val,
                            ::core::option::Option::None,
                        );
                    }
                }
            };
        }
        let v0 = {
            let val = inputs.remove(0);
            (*val.downcast_ref::<i32>().expect("pack downcast failed")).clone()
        };
        let v1 = {
            let val = inputs.remove(0);
            (*val.downcast_ref::<i32>().expect("pack downcast failed")).clone()
        };
        ::namu::__macro_exports::Value::new((v0, v1))
    }
    #[allow(dead_code)]
    pub fn unpack(
        val: ::namu::__macro_exports::Value,
    ) -> Vec<::namu::__macro_exports::Value> {
        <[_]>::into_vec(::alloc::boxed::box_new([val]))
    }
}
#[allow(non_snake_case)]
pub fn add<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
    b: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "add",
        <[_]>::into_vec(::alloc::boxed::box_new([a.id, b.id])),
    )
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
                        <[_]>::into_vec(
                            ::alloc::boxed::box_new([
                                (__while_parent_predecessor_0, __pre_while_a_0.id),
                            ]),
                        ),
                    );
                ::namu::__macro_exports::TracedValue::new(__phi_id)
            };
            a = __a_phi_val_0;
            let __a_phi_node_id_0 = __builder.arena().nodes.len() - 1;
            let __b_phi_val_0 = {
                let __phi_id = __builder
                    .phi(
                        <[_]>::into_vec(
                            ::alloc::boxed::box_new([
                                (__while_parent_predecessor_0, __pre_while_b_0.id),
                            ]),
                        ),
                    );
                ::namu::__macro_exports::TracedValue::new(__phi_id)
            };
            b = __b_phi_val_0;
            let __b_phi_node_id_0 = __builder.arena().nodes.len() - 1;
            let __i_phi_val_0 = {
                let __phi_id = __builder
                    .phi(
                        <[_]>::into_vec(
                            ::alloc::boxed::box_new([
                                (__while_parent_predecessor_0, __pre_while_i_0.id),
                            ]),
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
                *from = <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        (__while_parent_predecessor_0, __pre_while_a_0.id),
                        (__body_predecessor_id_0, __post_body_a_0.id),
                    ]),
                );
            }
            if let Some(
                ::namu::__macro_exports::Node {
                    kind: ::namu::__macro_exports::NodeKind::Phi { from },
                    ..
                },
            ) = __builder.arena_mut().nodes.get_mut(__b_phi_node_id_0)
            {
                *from = <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        (__while_parent_predecessor_0, __pre_while_b_0.id),
                        (__body_predecessor_id_0, __post_body_b_0.id),
                    ]),
                );
            }
            if let Some(
                ::namu::__macro_exports::Node {
                    kind: ::namu::__macro_exports::NodeKind::Phi { from },
                    ..
                },
            ) = __builder.arena_mut().nodes.get_mut(__i_phi_node_id_0)
            {
                *from = <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        (__while_parent_predecessor_0, __pre_while_i_0.id),
                        (__body_predecessor_id_0, __post_body_i_0.id),
                    ]),
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
