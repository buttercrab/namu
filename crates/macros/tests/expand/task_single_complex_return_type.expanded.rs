use macros::task;
pub struct MyComplexType {
    pub value: String,
}
fn __impl_complex_return_task(a: i32) -> anyhow::Result<MyComplexType> {
    Ok(MyComplexType {
        value: a.to_string(),
    })
}
#[allow(non_camel_case_types)]
struct __complex_return_task;
impl task::Task for __complex_return_task {
    type Config = ();
    type Input = i32;
    type Output = MyComplexType;
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
impl task::SingleTask for __complex_return_task {
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        let result = __impl_complex_return_task(a)?;
        Ok(result)
    }
}
fn __factory_complex_return_task() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let mut task_instance = __complex_return_task;
            let result = task::SingleTask::call(&mut task_instance, a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn complex_return_task<T: Clone + 'static>(
    builder: &graph::Builder<T>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<MyComplexType> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_complex_return_task: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_complex_return_task
        .call_once(|| {
            graph::register_task(
                "complex_return_task::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
                    .to_string(),
                __factory_complex_return_task(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "complex_return_task",
        task_id: "complex_return_task::/home/jaeyong/dev/github/namu/crates/macros/src/task.rs"
            .to_string(),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
