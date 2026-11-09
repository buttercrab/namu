use namu_macros::task;
#[doc(hidden)]
#[allow(non_camel_case_types)]
struct multiple_args_task {}
impl ::namu::__macro_exports::Raises for multiple_args_task {
    type Error = ::std::string::String;
    const TYPED: bool = false;
}
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
