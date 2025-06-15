use macros::{task, workflow};
fn __impl_task_a(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __task_a;
impl<Id, C> task::Task<Id, C> for __task_a
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
impl<Id, C> task::SingleTask<Id, C> for __task_a
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_task_a(a)
    }
}
fn __factory_task_a() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let result = __impl_task_a(a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn task_a<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_task_a: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_task_a
        .call_once(|| {
            graph::register_task(
                "task_a::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_task_a(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "task_a",
        task_id: "task_a::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_task_b(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __task_b;
impl<Id, C> task::Task<Id, C> for __task_b
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
impl<Id, C> task::SingleTask<Id, C> for __task_b
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_task_b(a)
    }
}
fn __factory_task_b() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let result = __impl_task_b(a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn task_b<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_task_b: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_task_b
        .call_once(|| {
            graph::register_task(
                "task_b::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_task_b(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "task_b",
        task_id: "task_b::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
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
