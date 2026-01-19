use namu_macros::{task, workflow};
#[allow(non_snake_case)]
pub mod is_positive {
    use super::*;
    fn task_impl(a: i32) -> anyhow::Result<bool> {
        Ok(a > 0)
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
        type Input = i32;
        type Output = bool;
        fn call(
            &mut self,
            input: Self::Input,
        ) -> ::namu::__macro_exports::Result<Self::Output> {
            let a = input;
            task_impl(a)
        }
    }
    #[allow(dead_code)]
    pub fn pack(
        mut inputs: Vec<::namu::__macro_exports::Value>,
    ) -> ::namu::__macro_exports::Value {
        if true {
            match (&inputs.len(), &1) {
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
        inputs.pop().unwrap()
    }
    #[allow(dead_code)]
    pub fn unpack(
        val: ::namu::__macro_exports::Value,
    ) -> Vec<::namu::__macro_exports::Value> {
        <[_]>::into_vec(::alloc::boxed::box_new([val]))
    }
}
#[allow(non_snake_case)]
pub fn is_positive<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<bool> {
    ::namu::__macro_exports::call(
        &builder,
        "is_positive",
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
#[allow(non_snake_case)]
pub mod action_if_true {
    use super::*;
    fn task_impl() -> anyhow::Result<()> {
        Ok(())
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
        type Input = ();
        type Output = ();
        fn call(
            &mut self,
            input: Self::Input,
        ) -> ::namu::__macro_exports::Result<Self::Output> {
            let () = input;
            task_impl()
        }
    }
    #[allow(dead_code)]
    pub fn pack(
        _inputs: Vec<::namu::__macro_exports::Value>,
    ) -> ::namu::__macro_exports::Value {
        ::namu::__macro_exports::Value::new(())
    }
    #[allow(dead_code)]
    pub fn unpack(
        val: ::namu::__macro_exports::Value,
    ) -> Vec<::namu::__macro_exports::Value> {
        <[_]>::into_vec(::alloc::boxed::box_new([val]))
    }
}
#[allow(non_snake_case)]
pub fn action_if_true<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
) -> ::namu::__macro_exports::TracedValue<()> {
    ::namu::__macro_exports::call(&builder, "action_if_true", ::alloc::vec::Vec::new())
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn if_with_task_in_condition_workflow() -> ::namu::__macro_exports::Graph<()> {
    let __builder = ::namu::__macro_exports::Builder::<()>::new();
    {
        let x = ::namu::__macro_exports::literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_condition = is_positive(&__builder, x);
            let __if_parent_predecessor_0 = __builder.current_block_id();
            ::namu::__macro_exports::branch(
                &__builder,
                __if_condition,
                __if_then_block_0,
                __if_merge_block_0,
            );
            __builder.switch_to_block(__if_then_block_0);
            let __then_val = {
                action_if_true(&__builder);
            };
            let __then_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __if_merge_block_0);
            __builder.switch_to_block(__if_merge_block_0);
        }
    };
    ::namu::__macro_exports::return_unit(&__builder);
    __builder.build()
}

fn __namu_build_pack() -> ::namu::__macro_exports::Workflow {
    pack().to_serializable("pack".to_string())
}

::namu::__macro_exports::inventory::submit! {
    ::namu::__macro_exports::WorkflowEntry {
        id: "pack",
        build: __namu_build_pack,
    }
}
