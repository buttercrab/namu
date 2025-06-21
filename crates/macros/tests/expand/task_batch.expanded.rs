use namu_macros::task;
fn __impl_batch_task(inputs: Vec<i32>) -> Vec<anyhow::Result<i32>> {
    inputs.into_iter().map(|i| Ok(i * 2)).collect()
}
#[allow(non_camel_case_types)]
pub struct __batch_task;
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::clone::Clone for __batch_task {
    #[inline]
    fn clone(&self) -> __batch_task {
        *self
    }
}
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::marker::Copy for __batch_task {}
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __batch_task
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
        ::namu::__macro_exports::BatchedTask::run(self, context)
    }
}
impl<Id, C> ::namu::__macro_exports::BatchedTask<Id, C> for __batch_task
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
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
        __impl_batch_task(input)
    }
}
#[allow(non_snake_case)]
pub fn batch_task<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    inputs: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "batch_task",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "batch_task",
                    "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/task_batch.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([inputs.id])),
    )
}
