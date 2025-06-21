use namu_macros::task;
fn __impl_stream_task(
    input: i32,
) -> anyhow::Result<impl Iterator<Item = anyhow::Result<i32>>> {
    Ok((0..input).map(Ok))
}
#[allow(non_camel_case_types)]
pub struct __stream_task;
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::clone::Clone for __stream_task {
    #[inline]
    fn clone(&self) -> __stream_task {
        *self
    }
}
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::marker::Copy for __stream_task {}
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __stream_task
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
        ::namu::__macro_exports::StreamTask::run(self, context)
    }
}
impl<Id, C> ::namu::__macro_exports::StreamTask<Id, C> for __stream_task
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    type Input = i32;
    type Output = i32;
    fn call(
        &mut self,
        input: Self::Input,
    ) -> impl Iterator<Item = ::namu::__macro_exports::Result<Self::Output>> {
        let input = input;
        __impl_stream_task(input).unwrap()
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
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "stream_task",
                    "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/task_stream.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([input.id])),
    )
}
