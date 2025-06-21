use namu_macros::{task, workflow};
fn __impl_task_a(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
pub struct __task_a;
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::clone::Clone for __task_a {
    #[inline]
    fn clone(&self) -> __task_a {
        *self
    }
}
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::marker::Copy for __task_a {}
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __task_a
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __task_a
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
        __impl_task_a(a)
    }
}
#[allow(non_snake_case)]
pub fn task_a<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "task_a",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "task_a",
                    "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/workflow_if_else_statement.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
fn __impl_task_b(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
pub struct __task_b;
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::clone::Clone for __task_b {
    #[inline]
    fn clone(&self) -> __task_b {
        *self
    }
}
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::marker::Copy for __task_b {}
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __task_b
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __task_b
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
        __impl_task_b(a)
    }
}
#[allow(non_snake_case)]
pub fn task_b<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    ::namu::__macro_exports::call(
        &builder,
        "task_b",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "task_b",
                    "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/workflow_if_else_statement.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn if_else_statement_workflow() -> ::namu::__macro_exports::Graph<()> {
    let __builder = ::namu::__macro_exports::Builder::<()>::new();
    {
        let x = ::namu::__macro_exports::literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_else_block_0 = __builder.new_block();
            let __if_condition = x > ::namu::__macro_exports::literal(&__builder, 20);
            let __if_parent_predecessor_0 = __builder.current_block_id();
            ::namu::__macro_exports::branch(
                &__builder,
                __if_condition,
                __if_then_block_0,
                __if_else_block_0,
            );
            __builder.switch_to_block(__if_then_block_0);
            let __then_val = {
                task_a(&__builder, x);
            };
            let __then_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __if_merge_block_0);
            __builder.switch_to_block(__if_else_block_0);
            let __else_val = {
                {
                    task_b(&__builder, x);
                }
            };
            let __else_predecessor_id_0 = __builder.current_block_id();
            ::namu::__macro_exports::jump(&__builder, __if_merge_block_0);
            __builder.switch_to_block(__if_merge_block_0);
        }
    };
    ::namu::__macro_exports::return_unit(&__builder);
    __builder.build()
}
