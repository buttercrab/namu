use namu_macros::task;
fn no_args_task<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
) -> ::namu::__macro_exports::TracedValue<()> {
    let __inputs = ::std::vec::Vec::from([]);
    ::namu::__macro_exports::call(__builder, "no_args_task", __inputs)
}
