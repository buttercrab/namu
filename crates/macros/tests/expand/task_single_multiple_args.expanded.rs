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
impl task::Task for __multiple_args_task {
    type Config = ();
    type Input = (i32, String);
    type Output = String;
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
impl task::SingleTask for __multiple_args_task {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let (a, b) = input;
        let result = __impl_multiple_args_task(a, b)?;
        Ok(result)
    }
}
fn __factory_multiple_args_task() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let b = __inputs[1usize].downcast_ref::<String>().unwrap().clone();
            let mut task_instance = __multiple_args_task;
            let result = task::SingleTask::call(&mut task_instance, (a, b)).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn multiple_args_task<T: Clone + 'static>(
    builder: &graph::Builder<T>,
    a: graph::TracedValue<i32>,
    b: graph::TracedValue<String>,
) -> graph::TracedValue<String> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_multiple_args_task: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_multiple_args_task
        .call_once(|| {
            graph::register_task(
                "multiple_args_task::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_multiple_args_task(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "multiple_args_task",
        task_id: "multiple_args_task::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id, b.id])),
    };
    builder.add_instruction(kind)
}
