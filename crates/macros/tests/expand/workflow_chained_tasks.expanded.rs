use macros::{task, workflow};
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
fn __impl_multiply_by_two(a: i32) -> anyhow::Result<i32> {
    Ok(a * 2)
}
#[allow(non_camel_case_types)]
struct __multiply_by_two;
impl task::Task for __multiply_by_two {
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
impl task::SingleTask for __multiply_by_two {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        let result = __impl_multiply_by_two(a)?;
        Ok(result)
    }
}
fn __factory_multiply_by_two() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let mut task_instance = __multiply_by_two;
            let result = task::SingleTask::call(&mut task_instance, a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn multiply_by_two<T: Clone + 'static>(
    builder: &graph::Builder<T>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_multiply_by_two: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_multiply_by_two
        .call_once(|| {
            graph::register_task(
                "multiply_by_two::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_multiply_by_two(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "multiply_by_two",
        task_id: "multiply_by_two::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn chained_tasks_workflow() -> graph::Graph<i32> {
    let __builder = graph::Builder::<i32>::new();
    let __result = {
        let initial = graph::new_literal(&__builder, 5);
        let added = add_one(&__builder, initial);
        multiply_by_two(&__builder, added)
    };
    __builder.seal_block(graph::Terminator::return_value(__result.id));
    __builder.build()
}
