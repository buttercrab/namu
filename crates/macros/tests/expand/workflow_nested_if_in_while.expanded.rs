use macros::{task, workflow};
fn __impl_less_than(a: i32, b: i32) -> anyhow::Result<bool> {
    Ok(a < b)
}
#[allow(non_camel_case_types)]
struct __less_than;
impl task::Task for __less_than {
    type Config = ();
    type Input = (i32, i32);
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
impl task::SingleTask for __less_than {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let (a, b) = input;
        let result = __impl_less_than(a, b)?;
        Ok(result)
    }
}
fn __factory_less_than() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let b = __inputs[1usize].downcast_ref::<i32>().unwrap().clone();
            let mut task_instance = __less_than;
            let result = task::SingleTask::call(&mut task_instance, (a, b)).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn less_than<T: Clone + 'static>(
    builder: &graph::Builder<T>,
    a: graph::TracedValue<i32>,
    b: graph::TracedValue<i32>,
) -> graph::TracedValue<bool> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_less_than: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_less_than
        .call_once(|| {
            graph::register_task(
                "less_than::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_less_than(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "less_than",
        task_id: "less_than::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id, b.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_is_even(a: i32) -> anyhow::Result<bool> {
    Ok(a % 2 == 0)
}
#[allow(non_camel_case_types)]
struct __is_even;
impl task::Task for __is_even {
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
impl task::SingleTask for __is_even {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        let result = __impl_is_even(a)?;
        Ok(result)
    }
}
fn __factory_is_even() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let mut task_instance = __is_even;
            let result = task::SingleTask::call(&mut task_instance, a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn is_even<T: Clone + 'static>(
    builder: &graph::Builder<T>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<bool> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_is_even: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_is_even
        .call_once(|| {
            graph::register_task(
                "is_even::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_is_even(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "is_even",
        task_id: "is_even::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_add_one(a: i32) -> anyhow::Result<i32> {
    Ok(a + 1)
}
#[allow(non_camel_case_types)]
struct __add_one;
impl task::Task for __add_one {
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
impl task::SingleTask for __add_one {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        let result = __impl_add_one(a)?;
        Ok(result)
    }
}
fn __factory_add_one() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let mut task_instance = __add_one;
            let result = task::SingleTask::call(&mut task_instance, a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn add_one<T: Clone + 'static>(
    builder: &graph::Builder<T>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_add_one: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_add_one
        .call_once(|| {
            graph::register_task(
                "add_one::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_add_one(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "add_one",
        task_id: "add_one::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_add_two(a: i32) -> anyhow::Result<i32> {
    Ok(a + 2)
}
#[allow(non_camel_case_types)]
struct __add_two;
impl task::Task for __add_two {
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
impl task::SingleTask for __add_two {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        let result = __impl_add_two(a)?;
        Ok(result)
    }
}
fn __factory_add_two() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let mut task_instance = __add_two;
            let result = task::SingleTask::call(&mut task_instance, a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn add_two<T: Clone + 'static>(
    builder: &graph::Builder<T>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_add_two: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_add_two
        .call_once(|| {
            graph::register_task(
                "add_two::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_add_two(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "add_two",
        task_id: "add_two::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
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
