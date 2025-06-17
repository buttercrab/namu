use namu_macros::{task, workflow};
fn __impl_less_than(a: i32, b: i32) -> anyhow::Result<bool> {
    Ok(a < b)
}
#[allow(non_camel_case_types)]
struct __less_than;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __less_than
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __less_than
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    type Input = (i32, i32);
    type Output = bool;
    fn call(
        &mut self,
        input: Self::Input,
    ) -> ::namu::__macro_exports::Result<Self::Output> {
        let (a, b) = input;
        __impl_less_than(a, b)
    }
}
#[allow(non_snake_case)]
pub fn less_than<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
    b: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<bool> {
    let kind = ::namu::__macro_exports::NodeKind::Call {
        task_name: "less_than",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "less_than",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_multiple_mutable_vars.rs",
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
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __add
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
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __add
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    type Input = (i32, i32);
    type Output = i32;
    fn call(
        &mut self,
        input: Self::Input,
    ) -> ::namu::__macro_exports::Result<Self::Output> {
        let (a, b) = input;
        __impl_add(a, b)
    }
}
#[allow(non_snake_case)]
pub fn add<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
    b: ::namu::__macro_exports::TracedValue<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    let kind = ::namu::__macro_exports::NodeKind::Call {
        task_name: "add",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "add",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/workflow_multiple_mutable_vars.rs",
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
pub fn multiple_mutable_vars_workflow() -> ::namu::__macro_exports::Graph<i32> {
    let __builder = ::namu::__macro_exports::Builder::<i32>::new();
    let __result = {
        let mut a = ::namu::__macro_exports::literal(&__builder, 0);
        let mut b = ::namu::__macro_exports::literal(&__builder, 1);
        let mut i = ::namu::__macro_exports::literal(&__builder, 0);
        {
            let __pre_while_a_0 = a;
            let __pre_while_b_0 = b;
            let __pre_while_i_0 = i;
            let __while_header_block_0 = __builder.new_block();
            let __while_body_block_0 = __builder.new_block();
            let __while_exit_block_0 = __builder.new_block();
            let __while_parent_predecessor_0 = __builder.current_block_id();
            __builder
                .seal_block(
                    ::namu::__macro_exports::Terminator::jump(__while_header_block_0),
                );
            __builder.switch_to_block(__while_header_block_0);
            let __a_phi_node_id_0 = __builder
                .arena_mut()
                .new_node(::namu::__macro_exports::NodeKind::Phi {
                    from: ::alloc::vec::Vec::new(),
                });
            a = ::namu::__macro_exports::TracedValue::new(__a_phi_node_id_0);
            __builder.add_instruction_to_current_block(__a_phi_node_id_0);
            let __b_phi_node_id_0 = __builder
                .arena_mut()
                .new_node(::namu::__macro_exports::NodeKind::Phi {
                    from: ::alloc::vec::Vec::new(),
                });
            b = ::namu::__macro_exports::TracedValue::new(__b_phi_node_id_0);
            __builder.add_instruction_to_current_block(__b_phi_node_id_0);
            let __i_phi_node_id_0 = __builder
                .arena_mut()
                .new_node(::namu::__macro_exports::NodeKind::Phi {
                    from: ::alloc::vec::Vec::new(),
                });
            i = ::namu::__macro_exports::TracedValue::new(__i_phi_node_id_0);
            __builder.add_instruction_to_current_block(__i_phi_node_id_0);
            let __while_cond = less_than(
                &__builder,
                i,
                ::namu::__macro_exports::literal(&__builder, 5),
            );
            __builder
                .seal_block(
                    ::namu::__macro_exports::Terminator::branch(
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
                i = add(&__builder, i, ::namu::__macro_exports::literal(&__builder, 1));
            };
            let __post_body_a_0 = a;
            let __post_body_b_0 = b;
            let __post_body_i_0 = i;
            let __body_predecessor_id_0 = __builder.current_block_id();
            __builder
                .seal_block(
                    ::namu::__macro_exports::Terminator::jump(__while_header_block_0),
                );
            if let Some(
                ::namu::__macro_exports::Node {
                    kind: ::namu::__macro_exports::NodeKind::Phi { from },
                    ..
                },
            ) = __builder.arena_mut().nodes.get_mut(__a_phi_node_id_0)
            {
                *from = <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        (__while_parent_predecessor_0, __pre_while_a_0.id),
                        (__body_predecessor_id_0, __post_body_a_0.id),
                    ]),
                );
            }
            if let Some(
                ::namu::__macro_exports::Node {
                    kind: ::namu::__macro_exports::NodeKind::Phi { from },
                    ..
                },
            ) = __builder.arena_mut().nodes.get_mut(__b_phi_node_id_0)
            {
                *from = <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        (__while_parent_predecessor_0, __pre_while_b_0.id),
                        (__body_predecessor_id_0, __post_body_b_0.id),
                    ]),
                );
            }
            if let Some(
                ::namu::__macro_exports::Node {
                    kind: ::namu::__macro_exports::NodeKind::Phi { from },
                    ..
                },
            ) = __builder.arena_mut().nodes.get_mut(__i_phi_node_id_0)
            {
                *from = <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        (__while_parent_predecessor_0, __pre_while_i_0.id),
                        (__body_predecessor_id_0, __post_body_i_0.id),
                    ]),
                );
            }
            __builder.switch_to_block(__while_exit_block_0);
            a = ::namu::__macro_exports::TracedValue::new(__a_phi_node_id_0);
            b = ::namu::__macro_exports::TracedValue::new(__b_phi_node_id_0);
            i = ::namu::__macro_exports::TracedValue::new(__i_phi_node_id_0);
        }
        a
    };
    __builder.seal_block(::namu::__macro_exports::Terminator::return_value(__result.id));
    __builder.build()
}
