use macros::{task, workflow};
fn __impl_double(a: i32) -> anyhow::Result<i32> {
    Ok(a * 2)
}
#[allow(non_camel_case_types)]
struct __double;
impl task::Task for __double {
    type Config = ();
    type Input = i32;
    type Output = i32;
    fn new(_config: Self::Config) -> Self {
        Self
    }
    fn run(
        &mut self,
        recv: task::Receiver<(usize, Self::Input)>,
        send: task::Sender<(usize, anyhow::Result<Self::Output>)>,
    ) {
        task::SingleTask::run(self, recv, send);
    }
}
impl task::SingleTask for __double {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        let result = __impl_double(a)?;
        Ok(result)
    }
}
fn __factory_double() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let mut task_instance = __double;
            let result = task::SingleTask::call(&mut task_instance, a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn double<T: Clone + 'static>(
    builder: &graph::Builder<T>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_double: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_double
        .call_once(|| {
            graph::register_task(
                "double::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_double(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "double",
        task_id: "double::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_identity(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __identity;
impl task::Task for __identity {
    type Config = ();
    type Input = i32;
    type Output = i32;
    fn new(_config: Self::Config) -> Self {
        Self
    }
    fn run(
        &mut self,
        recv: task::Receiver<(usize, Self::Input)>,
        send: task::Sender<(usize, anyhow::Result<Self::Output>)>,
    ) {
        task::SingleTask::run(self, recv, send);
    }
}
impl task::SingleTask for __identity {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        let result = __impl_identity(a)?;
        Ok(result)
    }
}
fn __factory_identity() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let mut task_instance = __identity;
            let result = task::SingleTask::call(&mut task_instance, a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn identity<T: Clone + 'static>(
    builder: &graph::Builder<T>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_identity: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_identity
        .call_once(|| {
            graph::register_task(
                "identity::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_identity(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "identity",
        task_id: "identity::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
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
