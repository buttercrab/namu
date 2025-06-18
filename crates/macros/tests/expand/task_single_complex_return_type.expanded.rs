use namu_macros::task;
pub struct MyComplexType {
    pub value: String,
}
fn __impl_complex_return_task(a: i32) -> anyhow::Result<MyComplexType> {
    Ok(MyComplexType {
        value: a.to_string(),
    })
}
#[allow(non_camel_case_types)]
struct __complex_return_task;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __complex_return_task
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __complex_return_task
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    type Input = i32;
    type Output = MyComplexType;
    fn call(
        &mut self,
        input: Self::Input,
    ) -> ::namu::__macro_exports::Result<Self::Output> {
        let a = input;
        __impl_complex_return_task(a)
    }
}
#[allow(non_snake_case)]
pub fn complex_return_task<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<MyComplexType> {
    ::namu::__macro_exports::call(
        &builder,
        "complex_return_task",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "complex_return_task",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/task_single_complex_return_type.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
