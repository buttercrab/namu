use namu_macros::task;
pub struct MyComplexType {
    pub value: String,
}
#[doc(hidden)]
#[allow(non_camel_case_types)]
struct complex_return_task {}
impl ::namu::__macro_exports::Raises for complex_return_task {
    type Error = ::std::string::String;
    const TYPED: bool = false;
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
