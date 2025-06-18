use namu_macros::task;
fn __impl_multiple_args_task(a: i32, b: String) -> anyhow::Result<String> {
    Ok(
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(format_args!("{0}{1}", a, b));
            res
        }),
    )
}
#[allow(non_camel_case_types)]
struct __multiple_args_task;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __multiple_args_task
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    fn prepare(&mut self) -> ::namu::__macro_exports::Result<()> {
        Ok(())
    }
    fn run(&mut self, context: C) -> ::namu::__macro_exports::Result<()> {
        ::namu::__macro_exports::SingleTask::run(self, context)
    }
}
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __multiple_args_task
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
        __impl_multiple_args_task(a, b)
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
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "multiple_args_task",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/task_single_multiple_args.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id, b.id])),
    )
}
