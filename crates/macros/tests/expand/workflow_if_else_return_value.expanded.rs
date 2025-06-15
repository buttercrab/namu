use macros::{task, workflow};
fn __impl_double(a: i32) -> anyhow::Result<i32> {
    Ok(a * 2)
}
#[allow(non_camel_case_types)]
struct __double;
impl<Id, C> task::Task<Id, C> for __double
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn run(&mut self, context: C) -> anyhow::Result<()> {
        task::SingleTask::run(self, context)
    }
}
impl<Id, C> task::SingleTask<Id, C> for __double
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_double(a)
    }
}
fn __factory_double() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let result = __impl_double(a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn double<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_double: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_double
        .call_once(|| {
            graph::register_task(
                ::alloc::__export::must_use({
                    let res = ::alloc::fmt::format(
                        format_args!(
                            "{0}::{1}", "double",
                            "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/workflow_if_else_return_value.rs",
                        ),
                    );
                    res
                }),
                __factory_double(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "double",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "double",
                    "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/workflow_if_else_return_value.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_identity(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __identity;
impl<Id, C> task::Task<Id, C> for __identity
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn run(&mut self, context: C) -> anyhow::Result<()> {
        task::SingleTask::run(self, context)
    }
}
impl<Id, C> task::SingleTask<Id, C> for __identity
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_identity(a)
    }
}
fn __factory_identity() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let result = __impl_identity(a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn identity<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_identity: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_identity
        .call_once(|| {
            graph::register_task(
                ::alloc::__export::must_use({
                    let res = ::alloc::fmt::format(
                        format_args!(
                            "{0}::{1}", "identity",
                            "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/workflow_if_else_return_value.rs",
                        ),
                    );
                    res
                }),
                __factory_identity(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "identity",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "identity",
                    "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/workflow_if_else_return_value.rs",
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
pub fn if_else_return_value_workflow() -> graph::Graph<i32> {
    let __builder = graph::Builder::<i32>::new();
    let __result = {
        let x = graph::new_literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_else_block_0 = __builder.new_block();
            let __if_condition = x > graph::new_literal(&__builder, 5);
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
            let __then_val = { double(&__builder, x) };
            let __then_predecessor_id_0 = __builder.current_block_id();
            __builder.seal_block(graph::Terminator::jump(__if_merge_block_0));
            __builder.switch_to_block(__if_else_block_0);
            let __else_val = { { identity(&__builder, x) } };
            let __else_predecessor_id_0 = __builder.current_block_id();
            __builder.seal_block(graph::Terminator::jump(__if_merge_block_0));
            __builder.switch_to_block(__if_merge_block_0);
            graph::phi(
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
    __builder.seal_block(graph::Terminator::return_value(__result.id));
    __builder.build()
}
