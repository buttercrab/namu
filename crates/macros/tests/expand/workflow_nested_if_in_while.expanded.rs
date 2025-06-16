use macros::{task, workflow};
fn __impl_less_than(a: i32, b: i32) -> anyhow::Result<bool> {
    Ok(a < b)
}
#[allow(non_camel_case_types)]
struct __less_than;
impl<Id> task::Task<Id> for __less_than
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
impl<Id> task::SingleTask<Id> for __less_than
where
    Id: Clone,
{
    type Input = (i32, i32);
    type Output = bool;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let (a, b) = input;
        __impl_less_than(a, b)
    }
}
#[allow(non_snake_case)]
pub fn less_than<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
    b: graph::TracedValue<i32>,
) -> graph::TracedValue<bool> {
    let kind = graph::NodeKind::Call {
        name: "less_than",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "less_than",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_nested_if_in_while.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id, b.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_is_even(a: i32) -> anyhow::Result<bool> {
    Ok(a % 2 == 0)
}
#[allow(non_camel_case_types)]
struct __is_even;
impl<Id> task::Task<Id> for __is_even
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
impl<Id> task::SingleTask<Id> for __is_even
where
    Id: Clone,
{
    type Input = i32;
    type Output = bool;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_is_even(a)
    }
}
#[allow(non_snake_case)]
pub fn is_even<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<bool> {
    let kind = graph::NodeKind::Call {
        name: "is_even",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "is_even",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_nested_if_in_while.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_add_one(a: i32) -> anyhow::Result<i32> {
    Ok(a + 1)
}
#[allow(non_camel_case_types)]
struct __add_one;
impl<Id> task::Task<Id> for __add_one
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
impl<Id> task::SingleTask<Id> for __add_one
where
    Id: Clone,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_add_one(a)
    }
}
#[allow(non_snake_case)]
pub fn add_one<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    let kind = graph::NodeKind::Call {
        name: "add_one",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "add_one",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_nested_if_in_while.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_add_two(a: i32) -> anyhow::Result<i32> {
    Ok(a + 2)
}
#[allow(non_camel_case_types)]
struct __add_two;
impl<Id> task::Task<Id> for __add_two
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
impl<Id> task::SingleTask<Id> for __add_two
where
    Id: Clone,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_add_two(a)
    }
}
#[allow(non_snake_case)]
pub fn add_two<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    let kind = graph::NodeKind::Call {
        name: "add_two",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "add_two",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_nested_if_in_while.rs",
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
pub fn nested_if_in_while_workflow() -> graph::Graph<i32> {
    let __builder = graph::Builder::<i32>::new();
    let __result = {
        let mut i = graph::new_literal(&__builder, 0);
        {
            let __pre_while_i_0 = i;
            let __while_header_block_0 = __builder.new_block();
            let __while_body_block_0 = __builder.new_block();
            let __while_exit_block_0 = __builder.new_block();
            let __while_parent_predecessor_0 = __builder.current_block_id();
            __builder.seal_block(graph::Terminator::jump(__while_header_block_0));
            __builder.switch_to_block(__while_header_block_0);
            let __i_phi_node_id_0 = __builder
                .arena_mut()
                .new_node(graph::NodeKind::Phi {
                    from: ::alloc::vec::Vec::new(),
                });
            i = graph::TracedValue::new(__i_phi_node_id_0);
            __builder.add_instruction_to_current_block(__i_phi_node_id_0);
            let __while_cond = less_than(
                &__builder,
                i,
                graph::new_literal(&__builder, 10),
            );
            __builder
                .seal_block(
                    graph::Terminator::branch(
                        __while_cond,
                        __while_body_block_0,
                        __while_exit_block_0,
                    ),
                );
            __builder.switch_to_block(__while_body_block_0);
            {
                {
                    let __pre_if_i_1 = i;
                    let __if_merge_block_1 = __builder.new_block();
                    let __if_then_block_1 = __builder.new_block();
                    let __if_else_block_1 = __builder.new_block();
                    let __if_condition = is_even(&__builder, i);
                    let __if_parent_predecessor_1 = __builder.current_block_id();
                    __builder
                        .seal_block(
                            graph::Terminator::branch(
                                __if_condition,
                                __if_then_block_1,
                                __if_else_block_1,
                            ),
                        );
                    __builder.switch_to_block(__if_then_block_1);
                    let __then_val = {
                        i = add_two(&__builder, i);
                    };
                    let __post_then_i_1 = i;
                    let __then_predecessor_id_1 = __builder.current_block_id();
                    __builder.seal_block(graph::Terminator::jump(__if_merge_block_1));
                    __builder.switch_to_block(__if_else_block_1);
                    let __else_val = {
                        i = __pre_if_i_1;
                        {
                            i = add_one(&__builder, i);
                        }
                    };
                    let __post_else_i_1 = i;
                    let __else_predecessor_id_1 = __builder.current_block_id();
                    __builder.seal_block(graph::Terminator::jump(__if_merge_block_1));
                    __builder.switch_to_block(__if_merge_block_1);
                    i = graph::phi(
                        &__builder,
                        <[_]>::into_vec(
                            ::alloc::boxed::box_new([
                                (__then_predecessor_id_1, __post_then_i_1),
                                (__else_predecessor_id_1, __post_else_i_1),
                            ]),
                        ),
                    );
                }
            };
            let __post_body_i_0 = i;
            let __body_predecessor_id_0 = __builder.current_block_id();
            __builder.seal_block(graph::Terminator::jump(__while_header_block_0));
            if let Some(graph::Node { kind: graph::NodeKind::Phi { from }, .. }) = __builder
                .arena_mut()
                .nodes
                .get_mut(__i_phi_node_id_0)
            {
                *from = <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        (__while_parent_predecessor_0, __pre_while_i_0.id),
                        (__body_predecessor_id_0, __post_body_i_0.id),
                    ]),
                );
            }
            __builder.switch_to_block(__while_exit_block_0);
            i = graph::TracedValue::new(__i_phi_node_id_0);
        }
        i
    };
    __builder.seal_block(graph::Terminator::return_value(__result.id));
    __builder.build()
}
