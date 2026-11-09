use namu_macros::task;
#[doc(hidden)]
#[allow(non_camel_case_types)]
struct single_arg_task {}
impl ::namu::__macro_exports::Raises for single_arg_task {
    type Error = ::std::string::String;
    const TYPED: bool = false;
}
fn single_arg_task<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<i32> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "single_arg_task", __inputs)
}
