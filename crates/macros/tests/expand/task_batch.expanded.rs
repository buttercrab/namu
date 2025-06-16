use macros::task;
fn __impl_batch_task(inputs: Vec<i32>) -> Vec<anyhow::Result<i32>> {
    inputs.into_iter().map(|i| Ok(i * 2)).collect()
}
#[allow(non_camel_case_types)]
struct __batch_task;
impl<Id> task::Task<Id> for __batch_task
where
    Id: Clone,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn run<C: task::TaskContext<Id>>(&mut self, context: C) -> anyhow::Result<()> {
        task::BatchedTask::run(self, context)
    }
}
impl<Id> task::BatchedTask<Id> for __batch_task
where
    Id: Clone,
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
#[allow(non_snake_case)]
pub fn batch_task<G: 'static>(
    builder: &graph::Builder<G>,
    inputs: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    let kind = graph::NodeKind::Call {
        name: "batch_task",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "batch_task",
                    "/home/coder/project/namu/crates/macros/tests/expand/task_batch.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([inputs.id])),
    };
    builder.add_instruction(kind)
}
