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
impl<Id> task::Task<Id> for __complex_return_task
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
impl<Id> task::SingleTask<Id> for __complex_return_task
where
    Id: Clone,
{
    type Input = i32;
    type Output = MyComplexType;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_complex_return_task(a)
    }
}
fn __factory_complex_return_task() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let result = __impl_complex_return_task(a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn complex_return_task<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<MyComplexType> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_complex_return_task: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_complex_return_task
        .call_once(|| {
            graph::register_task(
                ::alloc::__export::must_use({
                    let res = ::alloc::fmt::format(
                        format_args!(
                            "{0}::{1}", "complex_return_task",
                            "/home/coder/project/namu/crates/macros/tests/expand/task_single_complex_return_type.rs",
                        ),
                    );
                    res
                }),
                __factory_complex_return_task(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "complex_return_task",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "complex_return_task",
                    "/home/coder/project/namu/crates/macros/tests/expand/task_single_complex_return_type.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
