use macros::{task, workflow};
fn __impl_do_nothing(a: i32) -> anyhow::Result<i32> {
    Ok(a)
}
#[allow(non_camel_case_types)]
struct __do_nothing;
impl<Id> task::Task<Id> for __do_nothing
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
impl<Id> task::SingleTask<Id> for __do_nothing
where
    Id: Clone,
{
    type Input = i32;
    type Output = i32;
    fn call(&mut self, input: Self::Input) -> anyhow::Result<Self::Output> {
        let a = input;
        __impl_do_nothing(a)
    }
}
fn __factory_do_nothing() -> graph::TaskFactory {
    std::sync::Arc::new(|| {
        std::sync::Arc::new(|__inputs| {
            let a = __inputs[0usize].downcast_ref::<i32>().unwrap().clone();
            let result = __impl_do_nothing(a).unwrap();
            std::sync::Arc::new(result) as graph::Value
        })
    })
}
#[allow(non_snake_case)]
pub fn do_nothing<G: 'static>(
    builder: &graph::Builder<G>,
    a: graph::TracedValue<i32>,
) -> graph::TracedValue<i32> {
    #[allow(non_upper_case_globals)]
    static __REG_ONCE_do_nothing: std::sync::Once = std::sync::Once::new();
    __REG_ONCE_do_nothing
        .call_once(|| {
            graph::register_task(
                ::alloc::__export::must_use({
                    let res = ::alloc::fmt::format(
                        format_args!(
                            "{0}::{1}", "do_nothing",
                            "/home/coder/project/namu/crates/macros/tests/expand/workflow_if_statement.rs",
                        ),
                    );
                    res
                }),
                __factory_do_nothing(),
            );
        });
    let kind = graph::NodeKind::Call {
        name: "do_nothing",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "do_nothing",
                    "/home/coder/project/namu/crates/macros/tests/expand/workflow_if_statement.rs",
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
pub fn if_statement_workflow() -> graph::Graph<()> {
    let __builder = graph::Builder::<()>::new();
    {
        let x = graph::new_literal(&__builder, 10);
        {
            let __if_merge_block_0 = __builder.new_block();
            let __if_then_block_0 = __builder.new_block();
            let __if_condition = x > graph::new_literal(&__builder, 5);
            let __if_parent_predecessor_0 = __builder.current_block_id();
            __builder
                .seal_block(
                    graph::Terminator::branch(
                        __if_condition,
                        __if_then_block_0,
                        __if_merge_block_0,
                    ),
                );
            __builder.switch_to_block(__if_then_block_0);
            let __then_val = {
                do_nothing(&__builder, x);
            };
            let __then_predecessor_id_0 = __builder.current_block_id();
            __builder.seal_block(graph::Terminator::jump(__if_merge_block_0));
            __builder.switch_to_block(__if_merge_block_0);
        }
    };
    __builder.seal_block(graph::Terminator::return_unit());
    __builder.build()
}
