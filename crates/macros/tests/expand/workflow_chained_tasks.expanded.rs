use macros::{task, workflow};
fn __impl_add_one(a: i32) -> anyhow::Result<i32> {
    Ok(a + 1)
}
#[allow(non_camel_case_types)]
struct __add_one;
impl<Id> task::Task<Id> for __add_one
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
impl<Id> task::SingleTask<Id> for __add_one
where
    Id: Clone,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_add_one(a)
    }
}
fn __factory_add_one() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let result = __impl_add_one(a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn add_one<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_add_one: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_add_one
        .call_once(|| {
            graph::register_task(
                ::alloc::__export::must_use({
                    let res = ::alloc::fmt::format(
                        format_args!(
                            "{0}::{1}", "add_one",
                            "/home/coder/project/namu/crates/macros/tests/expand/workflow_chained_tasks.rs",
                        ),
                    );
                    res
                }),
                __factory_add_one(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "add_one",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "add_one",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_chained_tasks.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
fn __impl_multiply_by_two(a: i32) -> anyhow::Result<i32> {
    Ok(a * 2)
}
#[allow(non_camel_case_types)]
struct __multiply_by_two;
impl<Id> task::Task<Id> for __multiply_by_two
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
impl<Id> task::SingleTask<Id> for __multiply_by_two
where
    Id: Clone,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_multiply_by_two(a)
    }
}
fn __factory_multiply_by_two() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let result = __impl_multiply_by_two(a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn multiply_by_two<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_multiply_by_two: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_multiply_by_two
        .call_once(|| {
            graph::register_task(
                ::alloc::__export::must_use({
                    let res = ::alloc::fmt::format(
                        format_args!(
                            "{0}::{1}", "multiply_by_two",
                            "/home/coder/project/namu/crates/macros/tests/expand/workflow_chained_tasks.rs",
                        ),
                    );
                    res
                }),
                __factory_multiply_by_two(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "multiply_by_two",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "multiply_by_two",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_chained_tasks.rs",
                ),
            );
            res
        }),
        inputs: <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    };
    builder.add_instruction(kind)
}
#[allow(unused_assignments)]
#[allow(unused_braces)]
pub fn chained_tasks_workflow() -> graph::Graph<i32> {
    let __builder = graph::Builder::<i32>::new();
    let __result = {
        let initial = graph::new_literal(&__builder, 5);
        let added = add_one(&__builder, initial);
        multiply_by_two(&__builder, added)
    };
    __builder.seal_block(graph::Terminator::return_value(__result.id));
    __builder.build()
}
