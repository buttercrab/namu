use namu_macros::task;
pub struct MyComplexType {
    pub value: String,
}
fn complex_return_task<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
) -> ::namu::__macro_exports::TracedValue<MyComplexType> {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
    ]);
    ::namu::__macro_exports::call(__builder, "complex_return_task", __inputs)
}
