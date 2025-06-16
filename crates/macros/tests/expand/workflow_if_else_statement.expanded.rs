use macros::{task, workflow};
fn __impl_task_a(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __task_a;
impl<Id> task::Task<Id> for __task_a
where
    Id: Clone,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn run<C: task::TaskContext<Id>>(&mut self, context: C) -> anyhow::Result<()> {
        task::SingleTask::run(self, context)
    }
}
impl<Id> task::SingleTask<Id> for __task_a
where
    Id: Clone,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_task_a(a)
    }
}
#[allow(non_snake_case)]
pub fn task_a<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    let kind = graph::NodeKind::Call {
        name: "task_a",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "task_a",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_if_else_statement.rs",
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
impl<Id> task::Task<Id> for __task_b
where
    Id: Clone,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn run<C: task::TaskContext<Id>>(&mut self, context: C) -> anyhow::Result<()> {
        task::SingleTask::run(self, context)
    }
}
impl<Id> task::SingleTask<Id> for __task_b
where
    Id: Clone,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_task_b(a)
    }
}
#[allow(non_snake_case)]
pub fn task_b<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    let kind = graph::NodeKind::Call {
        name: "task_b",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "task_b",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_if_else_statement.rs",
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
pub fn if_else_statement_workflow() -> graph::Graph<()> {
    let __builder = graph::Builder::<()>::new();
    {
        let x = graph::new_literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_else_block_0 = __builder.new_block();
            let __if_condition = x > graph::new_literal(&__builder, 20);
            let __if_parent_predecessor_0 = __builder.current_block_id();
            __builder
                .seal_block(
                    graph::Terminator::branch(
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
            __builder.seal_block(graph::Terminator::jump(__if_merge_block_0));
            __builder.switch_to_block(__if_else_block_0);
            let __else_val = {
                {
                    task_b(&__builder, x);
                }
            };
            let __else_predecessor_id_0 = __builder.current_block_id();
            __builder.seal_block(graph::Terminator::jump(__if_merge_block_0));
            __builder.switch_to_block(__if_merge_block_0);
        }
    };
    __builder.seal_block(graph::Terminator::return_unit());
    __builder.build()
}
