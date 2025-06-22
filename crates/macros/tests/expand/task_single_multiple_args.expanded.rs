use namu_macros::task;
#[allow(non_snake_case)]
pub mod multiple_args_task {
    use super::*;
    fn task_impl(a: i32, b: String) -> anyhow::Result<String> {
        Ok(
            ::alloc::__export::must_use({
                let res = ::alloc::fmt::format(format_args!("{0}{1}", a, b));
                res
            }),
        )
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
        type Input = (i32, String);
        type Output = String;
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
            (*val.downcast_ref::<String>().expect("pack downcast failed")).clone()
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
pub fn multiple_args_task<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
    b: ::namu::__macro_exports::TracedValue<String>,
) -> ::namu::__macro_exports::TracedValue<String> {
    ::namu::__macro_exports::call(
        &builder,
        "multiple_args_task",
        <[_]>::into_vec(::alloc::boxed::box_new([a.id, b.id])),
    )
}
