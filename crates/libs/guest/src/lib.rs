//! SDK for writing NAMU tasks.
//!
//! The same task source compiles to either runtime:
//! - `--target wasm32-wasip2`: a WASM component implementing the `namu:task` WIT world.
//! - native (`crate-type = ["cdylib"]`): a shared library exporting the NAMU C ABI.
//!
//! ```ignore
//! namu_guest::task! {
//!     fn analyze(doc: String, min_len: i32) -> anyhow::Result<Stats> {
//!         Ok(Stats::of(&doc, min_len))
//!     }
//! }
//! ```
//!
//! One task per crate: the exported symbols have fixed names.

#[cfg(target_family = "wasm")]
pub mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "task",
        pub_export_macro: true,
        default_bindings_module: "::namu_guest::bindings",
    });
}

/// Version of the native C ABI exported by [`task!`].
pub const NATIVE_ABI_VERSION: u32 = 2;

#[doc(hidden)]
pub mod __private {
    pub use serde_json;

    /// Runs a task body against a JSON input and encodes its result as JSON.
    pub fn run<T, E, F>(input: &str, body: F) -> Result<String, String>
    where
        T: serde::Serialize,
        E: std::fmt::Display,
        F: FnOnce(&str) -> Result<Result<T, E>, serde_json::Error>,
    {
        let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(input)))
            .map_err(|panic| format!("task panicked: {}", panic_message(&panic)))?;
        match output {
            Ok(Ok(value)) => serde_json::to_string(&value).map_err(|err| err.to_string()),
            Ok(Err(err)) => Err(err.to_string()),
            Err(err) => Err(format!("invalid task input: {err}")),
        }
    }

    fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = panic.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        }
    }

    /// Native ABI entry point shared by every native task.
    ///
    /// # Safety
    /// `in_ptr` must point to `in_len` readable bytes; `out_ptr` and `out_len` must be writable.
    pub unsafe fn native_call(
        in_ptr: *const u8,
        in_len: usize,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
        call: fn(&str) -> Result<String, String>,
    ) -> i32 {
        let input = unsafe { std::slice::from_raw_parts(in_ptr, in_len) };
        let (code, payload) = match std::str::from_utf8(input) {
            Ok(input) => match call(input) {
                Ok(output) => (0, output),
                Err(err) => (1, err),
            },
            Err(err) => (1, format!("input is not UTF-8: {err}")),
        };
        // The host copies the output, then hands the buffer back through `namu_task_free`.
        let mut bytes = payload.into_bytes().into_boxed_slice();
        unsafe {
            *out_len = bytes.len();
            *out_ptr = bytes.as_mut_ptr();
        }
        std::mem::forget(bytes);
        code
    }

    /// # Safety
    /// `ptr`/`len` must come from a previous [`native_call`] output.
    pub unsafe fn native_free(ptr: *mut u8, len: usize) {
        if !ptr.is_null() {
            drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
        }
    }
}

/// Defines a NAMU task. See the crate docs.
#[macro_export]
macro_rules! task {
    (fn $name:ident() -> $ret:ty $body:block) => {
        fn $name() -> $ret $body
        $crate::__export!(|input| {
            let () = $crate::__private::serde_json::from_str::<Option<()>>(input).map(|_| ())?;
            Ok($name())
        });
    };
    (fn $name:ident($arg:ident : $ty:ty $(,)?) -> $ret:ty $body:block) => {
        fn $name($arg: $ty) -> $ret $body
        $crate::__export!(|input| {
            let $arg: $ty = $crate::__private::serde_json::from_str(input)?;
            Ok($name($arg))
        });
    };
    (fn $name:ident($($arg:ident : $ty:ty),+ $(,)?) -> $ret:ty $body:block) => {
        fn $name($($arg: $ty),+) -> $ret $body
        $crate::__export!(|input| {
            let ($($arg,)+): ($($ty,)+) = $crate::__private::serde_json::from_str(input)?;
            Ok($name($($arg),+))
        });
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __export {
    ($decode:expr) => {
        fn __namu_call(input: &str) -> ::std::result::Result<String, String> {
            $crate::__private::run(input, $decode)
        }

        #[cfg(target_family = "wasm")]
        const _: () = {
            struct NamuTask;
            impl $crate::bindings::Guest for NamuTask {
                fn call(input: String) -> ::std::result::Result<String, String> {
                    __namu_call(&input)
                }
            }
    $crate::bindings::export!(NamuTask with_types_in $crate::bindings);
        };

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub extern "C" fn namu_abi_version() -> u32 {
            $crate::NATIVE_ABI_VERSION
        }

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_task_call(
            in_ptr: *const u8,
            in_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            unsafe { $crate::__private::native_call(in_ptr, in_len, out_ptr, out_len, __namu_call) }
        }

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_task_free(ptr: *mut u8, len: usize) {
            unsafe { $crate::__private::native_free(ptr, len) }
        }
    };
}
