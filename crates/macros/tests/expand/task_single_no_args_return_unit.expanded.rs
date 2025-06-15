use macros::task;
fn __impl_no_args_task() -> anyhow::Result<()> {
    Ok(())
}
#[allow(non_camel_case_types)]
struct __no_args_task;
impl<Id, C> task::Task<Id, C> for __no_args_task
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
impl<Id, C> task::SingleTask<Id, C> for __no_args_task
where
    Id: Clone,
    C: task::TaskContext<Id>,
{
    type Input = ();
    type Output = ();
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let () = input;
        __impl_no_args_task()
    }
}
fn __factory_no_args_task() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let result = __impl_no_args_task().unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn no_args_task<G: 'static>(builder: &graph::Builder<G>) -> graph::TracedValue<()> {
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
