use macros::{task, workflow};
fn __impl_is_positive(a: i32) -> anyhow::Result<bool> {
    Ok(a > 0)
}
#[allow(non_camel_case_types)]
struct __is_positive;
impl<Id> task::Task<Id> for __is_positive
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
impl<Id> task::SingleTask<Id> for __is_positive
where
    Id: Clone,
{
    type Input = i32;
    type Output = bool;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_is_positive(a)
    }
}
#[allow(non_snake_case)]
pub fn is_positive<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<bool> {
    let kind = graph::NodeKind::Call {
        name: "is_positive",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "is_positive",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_if_with_task_in_condition.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_action_if_true() -> anyhow::Result<()> {
    Ok(())
}
#[allow(non_camel_case_types)]
struct __action_if_true;
impl<Id> task::Task<Id> for __action_if_true
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
impl<Id> task::SingleTask<Id> for __action_if_true
where
    Id: Clone,
{
    type Input = ();
    type Output = ();
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let () = input;
        __impl_action_if_true()
    }
}
#[allow(non_snake_case)]
pub fn action_if_true<G: 'static>(
    builder: &graph::Builder<G>,
) -> graph::TracedValue<()> {
    let kind = graph::NodeKind::Call {
        name: "action_if_true",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "action_if_true",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_if_with_task_in_condition.rs",
                ),
            );
            res
        }),
        inputs: ::alloc::vec::Vec::new(),
    };
    builder.add_instruction(kind)
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn if_with_task_in_condition_workflow() -> graph::Graph<()> {
    let __builder = graph::Builder::<()>::new();
    {
        let x = graph::new_literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_condition = is_positive(&__builder, x);
            let __if_parent_predecessor_0 = __builder.current_block_id();
            __builder
                .seal_block(
                    graph::Terminator::branch(
                        __if_condition,
                        __if_then_block_0,
                        __if_merge_block_0,
                    ),
                );
            __builder.switch_to_block(__if_then_block_0);
            let __then_val = {
                action_if_true(&__builder);
            };
            let __then_predecessor_id_0 = __builder.current_block_id();
            __builder.seal_block(graph::Terminator::jump(__if_merge_block_0));
            __builder.switch_to_block(__if_merge_block_0);
        }
    };
    __builder.seal_block(graph::Terminator::return_unit());
    __builder.build()
}
