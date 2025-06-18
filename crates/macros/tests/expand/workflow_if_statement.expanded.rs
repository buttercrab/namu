use namu_macros::{task, workflow};
fn __impl_do_nothing(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __do_nothing;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __do_nothing
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __do_nothing
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
        __impl_do_nothing(a)
    }
}
#[allow(non_snake_case)]
pub fn do_nothing<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "do_nothing",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "do_nothing",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_if_statement.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn if_statement_workflow() -> ::namu::__macro_exports::Graph<()> {
    let __builder = ::namu::__macro_exports::Builder::<()>::new();
    {
        let x = ::namu::__macro_exports::literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_condition = x > ::namu::__macro_exports::literal(&__builder, 5);
            let __if_parent_predecessor_0 = __builder.current_block_id();
            ::namu::__macro_exports::seal_block_branch(
                &__builder,
                __if_condition,
                __if_then_block_0,
                __if_merge_block_0,
            );
            __builder.switch_to_block(__if_then_block_0);
            let __then_val = {
                do_nothing(&__builder, x);
            };
            let __then_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::seal_block_jump(&__builder, __if_merge_block_0);
            __builder.switch_to_block(__if_merge_block_0);
        }
    };
    ::namu::__macro_exports::seal_block_return_unit(&__builder);
    __builder.build()
}
