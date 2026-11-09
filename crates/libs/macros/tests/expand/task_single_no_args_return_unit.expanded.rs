use namu_macros::task;
#[doc(hidden)]
#[allow(non_camel_case_types)]
struct no_args_task {}
impl ::namu::__macro_exports::Raises for no_args_task {
    type Error = ::std::string::String;
    const TYPED: bool = false;
}
fn no_args_task<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
) -> ::namu::__macro_exports::TracedValue<()> {
    let __inputs = ::std::vec::Vec::from([]);
    ::namu::__macro_exports::call(__builder, "no_args_task", __inputs)
}
