use macros::task;
fn __impl_stream_task(
    input: i32,
) -> anyhow::Result<impl Iterator<Item = anyhow::Result<i32>>> {
    Ok((0..input).map(Ok))
}
#[allow(non_camel_case_types)]
struct __stream_task;
impl<Id, C> task::Task<Id, C> for __stream_task
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn run(&mut self, context: C) -> anyhow::Result<()> {
        task::StreamTask::run(self, context)
    }
}
impl<Id, C> task::StreamTask<Id, C> for __stream_task
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    type Input = i32;
    type Output = i32;
    fn call(
        &mut self,
        input: Self::Input,
    ) -> impl Iterator<Item = anyhow::Result<Self::Output>> {
        let input = input;
        __impl_stream_task(input).unwrap()
    }
}
fn __factory_stream_task() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let input = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let result_iter = __impl_stream_task(input).unwrap();
            let result_vec: Vec<_> = result_iter.map(|item| item.unwrap()).collect();
            std::sync::Arc::new(result_vec) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn stream_task<G: 'static>(
    builder: &graph::Builder<G>,
    input: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_stream_task: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_stream_task
        .call_once(|| {
            graph::register_task(
                "stream_task::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_stream_task(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "stream_task",
        task_id: "stream_task::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([input.id])),
    };
    builder.add_instruction(kind)
}
