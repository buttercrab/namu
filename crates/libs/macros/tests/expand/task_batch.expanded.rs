use namu_macros::task;
#[allow(non_snake_case)]
pub mod batch_task {
    use super::*;
    fn task_impl(inputs: Vec<i32>) -> Vec<anyhow::Result<i32>> {
        inputs.into_iter().map(|i| Ok(i * 2)).collect()
    }
    #[allow(non_camel_case_types)]
    pub struct Task;
    #[automatically_derived]
    #[doc(hidden)]
    #[allow(non_camel_case_types)]
    unsafe impl ::core::clone::TrivialClone for Task {}
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
            ::namu::__macro_exports::BatchedTask::run(self, context)
        }
    }
    impl<C> ::namu::__macro_exports::BatchedTask<C> for Task
    where
        C: ::namu::__macro_exports::TaskContext,
    {
        type Input = i32;
        type Output = i32;
        fn batch_size(&self) -> usize {
            16usize
        }
        fn call(
            &mut self,
            input: Vec<Self::Input>,
        ) -> Vec<::namu::__macro_exports::Result<Self::Output>> {
            task_impl(input)
        }
    }
    #[allow(dead_code)]
    pub fn pack(
        mut inputs: Vec<::namu::__macro_exports::Value>,
    ) -> ::namu::__macro_exports::Value {
        if true {
            {
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
                }
            };
        }
        inputs.pop().unwrap()
    }
    #[allow(dead_code)]
    pub fn unpack(
        val: ::namu::__macro_exports::Value,
    ) -> Vec<::namu::__macro_exports::Value> {
        ::alloc::boxed::box_assume_init_into_vec_unsafe(
            ::alloc::intrinsics::write_box_via_move(
                ::alloc::boxed::Box::new_uninit(),
                [val],
            ),
        )
    }
}
#[allow(non_snake_case)]
pub fn batch_task<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    inputs: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    let __namu_inputs = ::alloc::boxed::box_assume_init_into_vec_unsafe(
        ::alloc::intrinsics::write_box_via_move(
            ::alloc::boxed::Box::new_uninit(),
            [::namu::__macro_exports::IntoArg::into_arg(inputs, builder)],
        ),
    );
    ::namu::__macro_exports::call(builder, "batch_task", __namu_inputs)
}
