use namu_macros::{task, workflow};
fn __impl_add_one(a: i32) -> anyhow::Result<i32> {
    Ok(a + 1)
}
#[allow(non_camel_case_types)]
struct __add_one;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __add_one
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __add_one
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
        __impl_add_one(a)
    }
}
#[allow(non_snake_case)]
pub fn add_one<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "add_one",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "add_one",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_chained_tasks.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
fn __impl_multiply_by_two(a: i32) -> anyhow::Result<i32> {
    Ok(a * 2)
}
#[allow(non_camel_case_types)]
struct __multiply_by_two;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __multiply_by_two
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __multiply_by_two
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
        __impl_multiply_by_two(a)
    }
}
#[allow(non_snake_case)]
pub fn multiply_by_two<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "multiply_by_two",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "multiply_by_two",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_chained_tasks.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn chained_tasks_workflow() -> ::namu::__macro_exports::Graph<i32> {
    let __builder = ::namu::__macro_exports::Builder::<i32>::new();
    let __result = {
        let initial = ::namu::__macro_exports::literal(&__builder, 5);
        let added = add_one(&__builder, initial);
        multiply_by_two(&__builder, added)
    };
    ::namu::__macro_exports::seal_block_return_value(&__builder, __result);
    __builder.build()
}
