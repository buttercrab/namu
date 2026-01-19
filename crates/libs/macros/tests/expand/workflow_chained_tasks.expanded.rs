use namu_macros::{task, workflow};
#[allow(non_snake_case)]
pub mod add_one {
    use super::*;
    fn task_impl(a: i32) -> anyhow::Result<i32> {
        Ok(a + 1)
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
        type Output = i32;
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
pub fn add_one<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "add_one",
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
#[allow(non_snake_case)]
pub mod multiply_by_two {
    use super::*;
    fn task_impl(a: i32) -> anyhow::Result<i32> {
        Ok(a * 2)
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
        type Output = i32;
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
pub fn multiply_by_two<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "multiply_by_two",
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn chained_tasks_workflow() -> ::namu::__macro_exports::Graph<i32> {
    let __builder = ::namu::__macro_exports::Builder::<i32>::new();
    let __result = {
        let initial = ::namu::__macro_exports::literal(&__builder, 5);
        let added = add_one(&__builder, initial);
        multiply_by_two(&__builder, added)
    };
    ::namu::__macro_exports::return_value(&__builder, __result);
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
