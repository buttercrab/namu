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
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_multiple_mutable_vars.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id, b.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_add(a: i32, b: i32) -> anyhow::Result<i32> {
    Ok(a + b)
}
#[allow(non_camel_case_types)]
struct __add;
impl<Id> task::Task<Id> for __add
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
impl<Id> task::SingleTask<Id> for __add
where
    Id: Clone,
{
    type Input = (i32, i32);
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let (a, b) = input;
        __impl_add(a, b)
    }
}
#[allow(non_snake_case)]
pub fn add<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
    b: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    let kind = graph::NodeKind::Call {
        name: "add",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "add",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_multiple_mutable_vars.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id, b.id])),
    };
    builder.add_instruction(kind)
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn multiple_mutable_vars_workflow() -> graph::Graph<i32> {
    let __builder = graph::Builder::<i32>::new();
    let __result = {
        let mut a = graph::new_literal(&__builder, 0);
        let mut b = graph::new_literal(&__builder, 1);
        let mut i = graph::new_literal(&__builder, 0);
        {
            let __pre_while_a_0 = a;
            let __pre_while_b_0 = b;
            let __pre_while_i_0 = i;
            let __while_header_block_0 = __builder.new_block();
            let __while_body_block_0 = __builder.new_block();
            let __while_exit_block_0 = __builder.new_block();
            let __while_parent_predecessor_0 = __builder.current_block_id();
            __builder.seal_block(graph::Terminator::jump(__while_header_block_0));
            __builder.switch_to_block(__while_header_block_0);
            let __a_phi_node_id_0 = __builder
                .arena_mut()
                .new_node(graph::NodeKind::Phi {
                    from: ::alloc::vec::Vec::new(),
                });
            a = graph::TracedValue::new(__a_phi_node_id_0);
            __builder.add_instruction_to_current_block(__a_phi_node_id_0);
            let __b_phi_node_id_0 = __builder
                .arena_mut()
                .new_node(graph::NodeKind::Phi {
                    from: ::alloc::vec::Vec::new(),
                });
            b = graph::TracedValue::new(__b_phi_node_id_0);
            __builder.add_instruction_to_current_block(__b_phi_node_id_0);
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
                graph::new_literal(&__builder, 5),
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
                let temp = a;
                a = b;
                b = add(&__builder, temp, b);
                i = add(&__builder, i, graph::new_literal(&__builder, 1));
            };
            let __post_body_a_0 = a;
            let __post_body_b_0 = b;
            let __post_body_i_0 = i;
            let __body_predecessor_id_0 = __builder.current_block_id();
            __builder.seal_block(graph::Terminator::jump(__while_header_block_0));
            if let Some(graph::Node { kind: graph::NodeKind::Phi { from }, .. }) = __builder
                .arena_mut()
                .nodes
                .get_mut(__a_phi_node_id_0)
            {
                *from = <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        (__while_parent_predecessor_0, __pre_while_a_0.id),
                        (__body_predecessor_id_0, __post_body_a_0.id),
                    ]),
                );
            }
            if let Some(graph::Node { kind: graph::NodeKind::Phi { from }, .. }) = __builder
                .arena_mut()
                .nodes
                .get_mut(__b_phi_node_id_0)
            {
                *from = <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        (__while_parent_predecessor_0, __pre_while_b_0.id),
                        (__body_predecessor_id_0, __post_body_b_0.id),
                    ]),
                );
            }
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
            a = graph::TracedValue::new(__a_phi_node_id_0);
            b = graph::TracedValue::new(__b_phi_node_id_0);
            i = graph::TracedValue::new(__i_phi_node_id_0);
        }
        a
    };
    __builder.seal_block(graph::Terminator::return_value(__result.id));
    __builder.build()
}
