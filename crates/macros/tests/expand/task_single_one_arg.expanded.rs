use namu_macros::task;
fn __impl_single_arg_task(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
pub struct __single_arg_task;
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::clone::Clone for __single_arg_task {
    #[inline]
    fn clone(&self) -> __single_arg_task {
        *self
    }
}
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::marker::Copy for __single_arg_task {}
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __single_arg_task
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __single_arg_task
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    type Input = i32;
    type Output = i32;
    fn call(
        &mut self,
        input: Self::Input,
    ) -> ::namu::__macro_exports::Result<Self::Output> {
        let a = input;
        __impl_single_arg_task(a)
    }
}
#[allow(non_snake_case)]
pub fn single_arg_task<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "single_arg_task",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "single_arg_task",
                    "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/task_single_one_arg.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
