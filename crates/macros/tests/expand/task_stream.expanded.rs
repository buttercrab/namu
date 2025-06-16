use macros::task;
fn __impl_stream_task(
    input: i32,
) -> anyhow::Result<impl Iterator<Item = anyhow::Result<i32>>> {
    Ok((0..input).map(Ok))
}
#[allow(non_camel_case_types)]
struct __stream_task;
impl<Id> task::Task<Id> for __stream_task
where
    Id: Clone,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn run<C: task::TaskContext<Id>>(&mut self, context: C) -> anyhow::Result<()> {
        task::StreamTask::run(self, context)
    }
}
impl<Id> task::StreamTask<Id> for __stream_task
where
    Id: Clone,
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
#[allow(non_snake_case)]
pub fn stream_task<G: 'static>(
    builder: &graph::Builder<G>,
    input: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    let kind = graph::NodeKind::Call {
        name: "stream_task",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "stream_task",
                    "/home/coder/project/namu/crates/macros/tests/expand/task_stream.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([input.id])),
    };
    builder.add_instruction(kind)
}
