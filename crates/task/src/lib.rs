/// A helper trait that allows a function to be called with a tuple of arguments.
/// This is the core of the system, enabling regular Rust functions to be treated as tasks.
pub trait Apply {
    type Input;
    type Output;

    fn apply(&self, input: Self::Input) -> Self::Output;
}

// This macro implements the `Apply` trait for functions with different numbers of arguments (from 0 to 12).
// This allows us to treat `fn(A, B)` as a type that can `apply` an input of `(A, B)`.
macro_rules! impl_apply {
    ($(($($arg:ident),*)),*) => {
        $(
            #[allow(non_snake_case)]
            impl<Func, Output, $($arg),*> Apply for Func
            where
                Func: Fn($($arg),*) -> Output,
            {
                type Input = ($($arg,)*);
                type Output = Output;

                #[inline]
                fn apply(&self, input: ($($arg,)*)) -> Self::Output {
                    let ($($arg,)*) = input;
                    self($($arg),*)
                }
            }
        )*
    };
}

// Generate implementations for functions with 0 to 12 arguments.
impl_apply! {
    (),
    (A),
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
    (A, B, C, D, E, F),
    (A, B, C, D, E, F, G),
    (A, B, C, D, E, F, G, H),
    (A, B, C, D, E, F, G, H, I),
    (A, B, C, D, E, F, G, H, I, J),
    (A, B, C, D, E, F, G, H, I, J, K),
    (A, B, C, D, E, F, G, H, I, J, K, L)
}

/// A trait representing a computational task.
///
/// This trait is automatically implemented for any function or closure that
/// takes up to 12 arguments, thanks to the `Apply` trait.
pub trait Task {
    type Input;
    /// The input arguments for the task, represented as a tuple.
    /// For a task with no inputs, this is `()`.
    /// For a task like `fn(i32, String)`, this will be `(i32, String)`.
    type Output;

    /// Runs the task with the given input tuple.
    fn run(&self, input: Self::Input) -> Self::Output;
}

/// A blanket implementation that turns any function that implements `Apply` into a `Task`.
///
/// This is the magic that makes the whole system work. For example, a function
/// `fn add(a: i32, b: i32) -> i32` will automatically implement `Task<(i32, i32)>`.
impl<Func, Input, Output> Task for Func
where
    Func: Apply<Input = Input, Output = Output>,
{
    type Output = Output;

    fn run(&self, input: Self::Input) -> Self::Output {
        self.apply(input)
    }
}
