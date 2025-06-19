use namu_macros::{task, workflow};
fn __impl_double(a: i32) -> anyhow::Result<i32> {
    Ok(a * 2)
}
#[allow(non_camel_case_types)]
struct __double;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __double
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __double
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
        __impl_double(a)
    }
}
#[allow(non_snake_case)]
pub fn double<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "double",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "double",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_if_else_return_value.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
fn __impl_identity(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __identity;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __identity
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __identity
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
        __impl_identity(a)
    }
}
#[allow(non_snake_case)]
pub fn identity<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "identity",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "identity",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_if_else_return_value.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn if_else_return_value_workflow() -> ::namu::__macro_exports::Graph<i32> {
    let __builder = ::namu::__macro_exports::Builder::<i32>::new();
    let __result = {
        let x = ::namu::__macro_exports::literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_else_block_0 = __builder.new_block();
            let __if_condition = x > ::namu::__macro_exports::literal(&__builder, 5);
            let __if_parent_predecessor_0 = __builder.current_block_id();
            ::namu::__macro_exports::branch(
                &__builder,
                __if_condition,
                __if_then_block_0,
                __if_else_block_0,
            );
            __builder.switch_to_block(__if_then_block_0);
            let __then_val = { double(&__builder, x) };
            let __then_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __if_merge_block_0);
            __builder.switch_to_block(__if_else_block_0);
            let __else_val = { { identity(&__builder, x) } };
            let __else_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __if_merge_block_0);
            __builder.switch_to_block(__if_merge_block_0);
            ::namu::__macro_exports::phi(
                &__builder,
                <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        (__then_predecessor_id_0, __then_val),
                        (__else_predecessor_id_0, __else_val),
                    ]),
                ),
            )
        }
    };
    ::namu::__macro_exports::return_value(&__builder, __result);
    __builder.build()
}
