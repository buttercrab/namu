use namu_macros::task;
#[allow(non_snake_case)]
pub mod no_args_task {
    use super::*;
    fn task_impl() -> anyhow::Result<()> {
        Ok(())
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
        ::alloc::boxed::box_assume_init_into_vec_unsafe(
            ::alloc::intrinsics::write_box_via_move(
                ::alloc::boxed::Box::new_uninit(),
                [val],
            ),
        )
    }
}
#[allow(non_snake_case)]
pub fn no_args_task<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
) -> ::namu::__macro_exports::TracedValue<()> {
    let __namu_inputs = ::alloc::vec::Vec::new();
    ::namu::__macro_exports::call(builder, "no_args_task", __namu_inputs)
}
