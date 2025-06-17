use namu_macros::task;
fn __impl_no_args_task() -> anyhow::Result<()> {
    Ok(())
}
#[allow(non_camel_case_types)]
struct __no_args_task;
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __no_args_task
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    fn prepare(&mut self) -> ::namu::__macro_exports::Result<()> {
        Ok(())
    }
    fn run(&mut self, context: C) -> ::namu::__macro_exports::Result<()> {
        ::namu::__macro_exports::SingleTask::run(self, context)
    }
}
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __no_args_task
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    type Input = ();
    type Output = ();
    fn call(
        &mut self,
        input: Self::Input,
    ) -> ::namu::__macro_exports::Result<Self::Output> {
        let () = input;
        __impl_no_args_task()
    }
}
#[allow(non_snake_case)]
pub fn no_args_task<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
) -> ::namu::__macro_exports::TracedValue<()> {
    let kind = ::namu::__macro_exports::NodeKind::Call {
        task_name: "no_args_task",
        task_id: ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "no_args_task",
                    "/Users/jaeyong/Development/Github/namu/crates/macros/tests/expand/task_single_no_args_return_unit.rs",
                ),
            );
            res
        }),
        inputs: ::alloc::vec::Vec::new(),
    };
    builder.add_instruction(kind)
}
