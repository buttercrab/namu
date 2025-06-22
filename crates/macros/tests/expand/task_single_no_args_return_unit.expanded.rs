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
pub fn no_args_task<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
) -> ::namu::__macro_exports::TracedValue<()> {
    ::namu::__macro_exports::call(
        &builder,
        "no_args_task",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "no_args_task",
                    "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/task_single_no_args_return_unit.rs",
                ),
            );
            res
        }),
        ::alloc::vec::Vec::new(),
    )
}
