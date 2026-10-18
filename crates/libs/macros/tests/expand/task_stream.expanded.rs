use namu_macros::task;
fn stream_task<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    input: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(input, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "stream_task", __inputs)
}
