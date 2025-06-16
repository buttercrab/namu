use macros::task;
fn __impl_single_arg_task(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __single_arg_task;
impl<Id> task::Task<Id> for __single_arg_task
where
    Id: Clone,
{
    fn prepare(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn run<C: task::TaskContext<Id>>(&mut self, context: C) -> anyhow::Result<()> {
        task::SingleTask::run(self, context)
    }
}
impl<Id> task::SingleTask<Id> for __single_arg_task
where
    Id: Clone,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_single_arg_task(a)
    }
}
fn __factory_single_arg_task() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let result = __impl_single_arg_task(a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn single_arg_task<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_single_arg_task: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_single_arg_task
        .call_once(|| {
            graph::register_task(
                ::alloc::__export::must_use({
                    let res = ::alloc::fmt::format(
                        format_args!(
                            "{0}::{1}", "single_arg_task",
                            "/home/coder/project/namu/crates/macros/tests/expand/task_single_one_arg.rs",
                        ),
                    );
                    res
                }),
                __factory_single_arg_task(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "single_arg_task",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "single_arg_task",
                    "/home/coder/project/namu/crates/macros/tests/expand/task_single_one_arg.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
