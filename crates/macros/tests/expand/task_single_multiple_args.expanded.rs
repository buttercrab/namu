use macros::task;
fn __impl_multiple_args_task(a: i32, b: String) -> anyhow::Result<String> {
    Ok(
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(format_args!("{0}{1}", a, b));
            res
        }),
    )
}
#[allow(non_camel_case_types)]
struct __multiple_args_task;
impl<Id> task::Task<Id> for __multiple_args_task
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
impl<Id> task::SingleTask<Id> for __multiple_args_task
where
    Id: Clone,
{
    type Input = (i32, String);
    type Output = String;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let (a, b) = input;
        __impl_multiple_args_task(a, b)
    }
}
fn __factory_multiple_args_task() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let b = __inputs[1usize].downcast_ref::<String>().unwrap().clone();
            let result = __impl_multiple_args_task(a, b).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn multiple_args_task<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
    b: graph::TracedValue<String>,
) -> graph::TracedValue<String> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_multiple_args_task: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_multiple_args_task
        .call_once(|| {
            graph::register_task(
                ::alloc::__export::must_use({
                    let res = ::alloc::fmt::format(
                        format_args!(
                            "{0}::{1}", "multiple_args_task",
                            "/home/coder/project/namu/crates/macros/tests/expand/task_single_multiple_args.rs",
                        ),
                    );
                    res
                }),
                __factory_multiple_args_task(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "multiple_args_task",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "multiple_args_task",
                    "/home/coder/project/namu/crates/macros/tests/expand/task_single_multiple_args.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id, b.id])),
    };
    builder.add_instruction(kind)
}
