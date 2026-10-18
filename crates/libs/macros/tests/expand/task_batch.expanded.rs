use namu_macros::task;
fn batch_task<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    inputs: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(inputs, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "batch_task", __inputs)
}
