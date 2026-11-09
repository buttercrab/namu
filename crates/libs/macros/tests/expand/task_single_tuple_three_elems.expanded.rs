use namu_macros::task;
#[doc(hidden)]
#[allow(non_camel_case_types)]
struct triple {}
impl ::namu::__macro_exports::Raises for triple {
    type Error = ::std::string::String;
    const TYPED: bool = false;
}
fn triple<G: 'static>(
    __builder: &::namu::__macro_exports::Builder<G>,
    a: impl ::namu::__macro_exports::IntoArg<i32>,
) -> (
    ::namu::__macro_exports::TracedValue<i32>,
    ::namu::__macro_exports::TracedValue<bool>,
    ::namu::__macro_exports::TracedValue<String>,
) {
    let __inputs = ::std::vec::Vec::from([
        ::namu::__macro_exports::IntoArg::into_arg(a, __builder),
    ]);
    ::namu::__macro_exports::call3(__builder, "triple", __inputs)
}
