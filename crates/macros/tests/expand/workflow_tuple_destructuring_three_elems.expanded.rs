use namu_macros::{task, workflow};
#[allow(non_snake_case)]
pub mod triple {
    use super::*;
    fn task_impl(a: i32) -> anyhow::Result<(i32, bool, String)> {
        Ok((a, a > 0, a.to_string()))
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
    impl<Id, C> ::namu::__macro_exports::Task<Id, C> for Task
    where
        Id: Clone,
        C: ::namu::__macro_exports::TaskContext<Id>,
    {
        fn prepare(&mut self) -> ::namu::__macro_exports::Result<()> {
            Ok(())
        }
        fn clone_boxed(
            &self,
        ) -> Box<dyn ::namu::__macro_exports::Task<Id, C> + Send + Sync> {
            Box::new(*self)
        }
        fn run(&mut self, context: C) -> ::namu::__macro_exports::Result<()> {
            ::namu::__macro_exports::SingleTask::run(self, context)
        }
    }
    impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for Task
    where
        Id: Clone,
        C: ::namu::__macro_exports::TaskContext<Id>,
    {
        type Input = i32;
        type Output = (i32, bool, String);
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
        let tuple_val: (i32, bool, String) = unsafe {
            val.take::<(i32, bool, String)>()
        };
        let (o0, o1, o2) = tuple_val;
        <[_]>::into_vec(
            ::alloc::boxed::box_new([
                ::namu::__macro_exports::Value::new(o0),
                ::namu::__macro_exports::Value::new(o1),
                ::namu::__macro_exports::Value::new(o2),
            ]),
        )
    }
}
#[allow(non_snake_case)]
pub fn triple<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> (
    ::namu::__macro_exports::TracedValue<i32>,
    ::namu::__macro_exports::TracedValue<bool>,
    ::namu::__macro_exports::TracedValue<String>,
) {
    ::namu::__macro_exports::call3(
        &builder,
        "triple",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "triple",
                    "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/workflow_tuple_destructuring_three_elems.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
