use macros::task;
fn __impl_no_args_task() -> anyhow::Result<()> {
    Ok(())
}
#[allow(non_camel_case_types)]
struct __no_args_task;
impl task::Task for __no_args_task {
    type Config = ();
    type Input = ();
    type Output = ();
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
impl task::SingleTask for __no_args_task {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let () = input;
        let result = __impl_no_args_task()?;
        Ok(result)
    }
}
fn __factory_no_args_task() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let mut task_instance = __no_args_task;
            let result = task::SingleTask::call(&mut task_instance, ()).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn no_args_task<T: Clone + 'static>(
    builder: &graph::Builder<T>,
) -> graph::TracedValue<()> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_no_args_task: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_no_args_task
        .call_once(|| {
            graph::register_task(
                "no_args_task::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_no_args_task(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "no_args_task",
        task_id: "no_args_task::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: ::alloc::vec::Vec::new(),
    };
    builder.add_instruction(kind)
}
