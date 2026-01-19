use namu_macros::task;
#[allow(non_snake_case)]
pub mod stream_task {
    use super::*;
    fn task_impl(
        input: i32,
    ) -> anyhow::Result<impl Iterator<Item = anyhow::Result<i32>>> {
        Ok((0..input).map(Ok))
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
            ::namu::__macro_exports::StreamTask::run(self, context)
        }
    }
    impl<C> ::namu::__macro_exports::StreamTask<C> for Task
    where
        C: ::namu::__macro_exports::TaskContext,
    {
        type Input = i32;
        type Output = i32;
        fn call(
            &mut self,
            input: Self::Input,
        ) -> impl Iterator<Item = ::namu::__macro_exports::Result<Self::Output>> {
            let input = input;
            task_impl(input).unwrap()
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
pub fn stream_task<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    input: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "stream_task",
        <[_]>::into_vec(::alloc::boxed::box_new([input.id])),
    )
}
