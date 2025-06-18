use namu_macros::{task, workflow};
fn __impl_is_positive(a: i32) -> anyhow::Result<bool> {
    Ok(a > 0)
}
#[allow(non_camel_case_types)]
struct __is_positive;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __is_positive
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __is_positive
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    type Input = i32;
    type Output = bool;
    fn call(
        &mut self,
        input: Self::Input,
    ) -> ::namu::__macro_exports::Result<Self::Output> {
        let a = input;
        __impl_is_positive(a)
    }
}
#[allow(non_snake_case)]
pub fn is_positive<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<bool> {
    ::namu::__macro_exports::call(
        &builder,
        "is_positive",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "is_positive",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_if_with_task_in_condition.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
fn __impl_action_if_true() -> anyhow::Result<()> {
    Ok(())
}
#[allow(non_camel_case_types)]
struct __action_if_true;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __action_if_true
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __action_if_true
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
        __impl_action_if_true()
    }
}
#[allow(non_snake_case)]
pub fn action_if_true<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
) -> ::namu::__macro_exports::TracedValue<()> {
    ::namu::__macro_exports::call(
        &builder,
        "action_if_true",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "action_if_true",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_if_with_task_in_condition.rs",
                ),
            );
            res
        }),
        ::alloc::vec::Vec::new(),
    )
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn if_with_task_in_condition_workflow() -> ::namu::__macro_exports::Graph<()> {
    let __builder = ::namu::__macro_exports::Builder::<()>::new();
    {
        let x = ::namu::__macro_exports::literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_condition = is_positive(&__builder, x);
            let __if_parent_predecessor_0 = __builder.current_block_id();
            ::namu::__macro_exports::seal_block_branch(
                &__builder,
                __if_condition,
                __if_then_block_0,
                __if_merge_block_0,
            );
            __builder.switch_to_block(__if_then_block_0);
            let __then_val = {
                action_if_true(&__builder);
            };
            let __then_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::seal_block_jump(&__builder, __if_merge_block_0);
            __builder.switch_to_block(__if_merge_block_0);
        }
    };
    ::namu::__macro_exports::seal_block_return_unit(&__builder);
    __builder.build()
}
