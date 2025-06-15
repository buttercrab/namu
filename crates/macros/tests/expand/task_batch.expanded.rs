use macros::task;
fn __impl_batch_task(inputs: Vec<i32>) -> Vec<anyhow::Result<i32>> {
    inputs.into_iter().map(|i| Ok(i * 2)).collect()
}
#[allow(non_camel_case_types)]
struct __batch_task;
impl<Id, C> task::Task<Id, C> for __batch_task
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn run(&mut self, context: C) -> anyhow::Result<()> {
        task::BatchedTask::run(self, context)
    }
}
impl<Id, C> task::BatchedTask<Id, C> for __batch_task
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    type Input = i32;
    type Output = i32;
    fn batch_size(&self) -> usize {
        16usize
    }
    fn call(&mut self, input: Vec<Self::Input>) -> Vec<anyhow::Result<Self::Output>> {
        __impl_batch_task(input)
    }
}
fn __factory_batch_task() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let inputs = __inputs[0].downcast_ref::<Vec<i32>>().unwrap().clone();
            let result = __impl_batch_task(inputs).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn batch_task<G: 'static>(
    builder: &graph::Builder<G>,
    inputs: graph::TracedValue<Vec<i32>>,
) -> graph::TracedValue<Vec<i32>> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_batch_task: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_batch_task
        .call_once(|| {
            graph::register_task(
                "batch_task::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_batch_task(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "batch_task",
        task_id: "batch_task::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([inputs.id])),
    };
    builder.add_instruction(kind)
}
