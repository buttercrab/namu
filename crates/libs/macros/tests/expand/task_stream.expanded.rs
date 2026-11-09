use namu_macros::task;
#[doc(hidden)]
#[allow(non_camel_case_types)]
struct stream_task {}
impl ::namu::__macro_exports::Raises for stream_task {
    type Error = ::std::string::String;
    const TYPED: bool = false;
}
fn stream_task<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    input: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<::std::vec::Vec<i32>> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(input, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "stream_task", __inputs)
}
