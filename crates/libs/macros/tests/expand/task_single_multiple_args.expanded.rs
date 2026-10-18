use namu_macros::task;
fn multiple_args_task<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
    b: impl ::namu::__macro_exports::IntoArg<String>,
) -> ::namu::__macro_exports::TracedValue<String> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
        ::namu::__macro_exports::IntoArg::into_arg(b, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "multiple_args_task", __inputs)
}
