use macros::{task, workflow};
fn __impl_is_positive(a: i32) -> anyhow::Result<bool> {
    Ok(a > 0)
}
#[allow(non_camel_case_types)]
struct __is_positive;
impl task::Task for __is_positive {
    type Config = ();
    type Input = i32;
    type Output = bool;
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
impl task::SingleTask for __is_positive {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        let result = __impl_is_positive(a)?;
        Ok(result)
    }
}
fn __factory_is_positive() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let mut task_instance = __is_positive;
            let result = task::SingleTask::call(&mut task_instance, a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn is_positive<T: Clone + 'static>(
    builder: &graph::Builder<T>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<bool> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_is_positive: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_is_positive
        .call_once(|| {
            graph::register_task(
                "is_positive::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_is_positive(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "is_positive",
        task_id: "is_positive::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_action_if_true() -> anyhow::Result<()> {
    Ok(())
}
#[allow(non_camel_case_types)]
struct __action_if_true;
impl task::Task for __action_if_true {
    type Config = ();
    type Input = ();
    type Output = ();
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
impl task::SingleTask for __action_if_true {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let () = input;
        let result = __impl_action_if_true()?;
        Ok(result)
    }
}
fn __factory_action_if_true() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let mut task_instance = __action_if_true;
            let result = task::SingleTask::call(&mut task_instance, ()).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn action_if_true<T: Clone + 'static>(
    builder: &graph::Builder<T>,
) -> graph::TracedValue<()> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_action_if_true: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_action_if_true
        .call_once(|| {
            graph::register_task(
                "action_if_true::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_action_if_true(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "action_if_true",
        task_id: "action_if_true::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
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
