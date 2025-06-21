use namu_macros::{task, workflow};
fn __impl_triple(a: i32) -> anyhow::Result<(i32, bool, String)> {
    Ok((a, a > 0, a.to_string()))
}
#[allow(non_camel_case_types)]
pub struct __triple;
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::clone::Clone for __triple {
    #[inline]
    fn clone(&self) -> __triple {
        *self
    }
}
#[automatically_derived]
#[allow(non_camel_case_types)]
impl ::core::marker::Copy for __triple {}
impl<Id, C> ::namu::__macro_exports::Task<Id, C> for __triple
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    fn prepare(&mut self) -> ::namu::__macro_exports::Result<()> {
        Ok(())
    }
    fn clone_boxed(
        &self,
    ) -> Box<dyn ::namu::__macro_exports::Task<Id, C> + Send + Sync> {
        Box::new(*self)
    }
    fn run(&mut self, context: C) -> ::namu::__macro_exports::Result<()> {
        ::namu::__macro_exports::SingleTask::run(self, context)
    }
}
impl<Id, C> ::namu::__macro_exports::SingleTask<Id, C> for __triple
where
    Id: Clone,
    C: ::namu::__macro_exports::TaskContext<Id>,
{
    type Input = i32;
    type Output = (i32, bool, String);
    fn call(
        &mut self,
        input: Self::Input,
    ) -> ::namu::__macro_exports::Result<Self::Output> {
        let a = input;
        __impl_triple(a)
    }
}
#[allow(non_snake_case)]
pub fn triple<G: 'static>(
    builder: &::namu::__macro_exports::Builder<G>,
    a: ::namu::__macro_exports::TracedValue<i32>,
) -> (
    ::namu::__macro_exports::TracedValue<i32>,
    ::namu::__macro_exports::TracedValue<bool>,
    ::namu::__macro_exports::TracedValue<String>,
) {
    ::namu::__macro_exports::call3(
        &builder,
        "triple",
        ::alloc::__export::must_use({
            let res = ::alloc::fmt::format(
                format_args!(
                    "{0}::{1}", "triple",
                    "/home/jaeyong/dev/github/namu/crates/macros/tests/expand/workflow_tuple_destructuring_three_elems.rs",
                ),
            );
            res
        }),
        <[_]>::into_vec(::alloc::boxed::box_new([a.id])),
    )
}
