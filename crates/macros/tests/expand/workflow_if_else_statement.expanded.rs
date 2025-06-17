use namu_macros::{task, workflow};
fn __impl_task_a(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __task_a;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __task_a
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
    let kind = ::namu::__macro_exports::NodeKind::Call {
        task_name: "task_a",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "task_a",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_if_else_statement.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_task_b(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __task_b;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __task_b
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
    let kind = ::namu::__macro_exports::NodeKind::Call {
        task_name: "task_b",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "task_b",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_if_else_statement.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
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
            __builder
                .seal_block(
                    ::namu::__macro_exports::Terminator::branch(
                        __if_condition,
                        __if_then_block_0,
                        __if_else_block_0,
                    ),
                );
            __builder.switch_to_block(__if_then_block_0);
            let __then_val = {
                task_a(&__builder, x);
            };
            let __then_predecessor_id_0 = __builder.current_block_id();
            __builder
                .seal_block(
                    ::namu::__macro_exports::Terminator::jump(__if_merge_block_0),
                );
            __builder.switch_to_block(__if_else_block_0);
            let __else_val = {
                {
                    task_b(&__builder, x);
                }
            };
            let __else_predecessor_id_0 = __builder.current_block_id();
            __builder
                .seal_block(
                    ::namu::__macro_exports::Terminator::jump(__if_merge_block_0),
                );
            __builder.switch_to_block(__if_merge_block_0);
        }
    };
    __builder.seal_block(::namu::__macro_exports::Terminator::return_unit());
    __builder.build()
}
