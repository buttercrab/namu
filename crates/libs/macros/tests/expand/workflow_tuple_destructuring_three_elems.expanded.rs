use namu_macros::{task, workflow};
fn triple<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
) -> (
    ::namu::__macro_exports::TracedValue<i32>,
    ::namu::__macro_exports::TracedValue<bool>,
    ::namu::__macro_exports::TracedValue<String>,
) {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
    ]);
    ::namu::__macro_exports::call3(__builder, "triple", __inputs)
}
