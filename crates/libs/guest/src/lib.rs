//! SDK for writing NAMU tasks.
//!
//! The same task source compiles to either runtime:
//! - `--target wasm32-wasip2`: a WASM component implementing the `namu:task` WIT world (`task`, or
//!   `stream-task` for a stream).
//! - native (`crate-type = ["cdylib"]`): a shared library exporting the NAMU C ABI.
//!
//! ```ignore
//! namu_guest::task! {
//!     fn analyze(doc: String, min_len: i32) -> anyhow::Result<Stats> {
//!         Ok(Stats::of(&doc, min_len))
//!     }
//! }
//!
//! namu_guest::stream! {
//!     fn pages(site: String) -> anyhow::Result<impl Iterator<Item = anyhow::Result<Page>>> {
//!         Ok(crawl(site))
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

#[cfg(target_family = "wasm")]
pub mod stream_bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "stream-task",
        pub_export_macro: true,
        export_macro_name: "export_stream",
        default_bindings_module: "::namu_guest::stream_bindings",
    });
}

/// Version of the native C ABI exported by [`task!`] and [`stream!`]. Version 3 added streams.
pub const NATIVE_ABI_VERSION: u32 = 3;

#[doc(hidden)]
pub mod __private {
    use std::any::Any;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    pub use serde_json;

    /// A stream's items, each JSON or the task's error.
    pub type Items = Box<dyn Iterator<Item = Result<String, String>>>;

    /// Runs a task body against a JSON input and encodes its result as JSON.
    pub fn run<T, E, F>(input: &str, body: F) -> Result<String, String>
    where
        T: serde::Serialize,
        E: std::fmt::Display,
        F: FnOnce(String) -> Result<Result<T, E>, serde_json::Error>,
    {
        let output = catch_unwind(AssertUnwindSafe(|| body(input.to_string())))
            .map_err(|panic| format!("task panicked: {}", panic_message(&panic)))?;
        match output {
            Ok(Ok(value)) => serde_json::to_string(&value).map_err(|err| err.to_string()),
            Ok(Err(err)) => Err(err.to_string()),
            Err(err) => Err(format!("invalid task input: {err}")),
        }
    }

    /// Opens a stream task against a JSON input; its items are encoded as JSON as they are pulled.
    pub fn open<T, E, I, F>(input: &str, body: F) -> Result<Items, String>
    where
        T: serde::Serialize,
        E: std::fmt::Display,
        I: Iterator<Item = Result<T, E>> + 'static,
        F: FnOnce(String) -> Result<Result<I, E>, serde_json::Error>,
    {
        let items = catch_unwind(AssertUnwindSafe(|| body(input.to_string())))
            .map_err(|panic| format!("task panicked: {}", panic_message(&panic)))?;
        let mut items = match items {
            Ok(Ok(items)) => items,
            Ok(Err(err)) => return Err(err.to_string()),
            Err(err) => return Err(format!("invalid task input: {err}")),
        };
        Ok(Box::new(std::iter::from_fn(move || {
            let item = catch_unwind(AssertUnwindSafe(|| items.next()));
            Some(match item {
                Ok(None) => return None,
                Ok(Some(Ok(value))) => serde_json::to_string(&value).map_err(|err| err.to_string()),
                Ok(Some(Err(err))) => Err(err.to_string()),
                Err(panic) => Err(format!("task panicked: {}", panic_message(&panic))),
            })
        })))
    }

    fn panic_message(panic: &Box<dyn Any + Send>) -> String {
        if let Some(s) = panic.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        }
    }

    /// Hands `payload` to the host, which copies it and gives it back through `namu_task_free`.
    ///
    /// # Safety
    /// `out_ptr` and `out_len` must be writable.
    unsafe fn hand_over(payload: String, out_ptr: *mut *mut u8, out_len: *mut usize) {
        let mut bytes = payload.into_bytes().into_boxed_slice();
        unsafe {
            *out_len = bytes.len();
            *out_ptr = bytes.as_mut_ptr();
        }
        std::mem::forget(bytes);
    }

    /// # Safety
    /// `in_ptr` must point to `in_len` readable bytes.
    unsafe fn input<'a>(in_ptr: *const u8, in_len: usize) -> Result<&'a str, String> {
        let input = unsafe { std::slice::from_raw_parts(in_ptr, in_len) };
        std::str::from_utf8(input).map_err(|err| format!("input is not UTF-8: {err}"))
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
        let (code, payload) = match unsafe { input(in_ptr, in_len) }.and_then(call) {
            Ok(output) => (0, output),
            Err(err) => (1, err),
        };
        unsafe { hand_over(payload, out_ptr, out_len) };
        code
    }

    /// Native ABI entry point that opens a stream. On success `handle` is set; on failure the
    /// error is handed over in `out_ptr`/`out_len`.
    ///
    /// # Safety
    /// `in_ptr` must point to `in_len` readable bytes; the out-pointers must be writable.
    pub unsafe fn native_open(
        in_ptr: *const u8,
        in_len: usize,
        handle: *mut *mut std::ffi::c_void,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
        open: fn(&str) -> Result<Items, String>,
    ) -> i32 {
        match unsafe { input(in_ptr, in_len) }.and_then(open) {
            Ok(items) => {
                unsafe { *handle = Box::into_raw(Box::new(items)).cast() };
                0
            }
            Err(err) => {
                unsafe { hand_over(err, out_ptr, out_len) };
                1
            }
        }
    }

    /// Native ABI entry point for a stream's next item: 0 and the item, 1 at the end, or 2 and
    /// the task's error.
    ///
    /// # Safety
    /// `handle` must come from [`native_open`] and not be closed; the out-pointers must be
    /// writable.
    pub unsafe fn native_next(
        handle: *mut std::ffi::c_void,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
    ) -> i32 {
        let items = unsafe { &mut *handle.cast::<Items>() };
        let (code, payload) = match items.next() {
            None => return 1,
            Some(Ok(item)) => (0, item),
            Some(Err(err)) => (2, err),
        };
        unsafe { hand_over(payload, out_ptr, out_len) };
        code
    }

    /// # Safety
    /// `handle` must come from [`native_open`] and not be closed yet.
    pub unsafe fn native_close(handle: *mut std::ffi::c_void) {
        drop(unsafe { Box::from_raw(handle.cast::<Items>()) });
    }

    /// # Safety
    /// `ptr`/`len` must come from a buffer handed over by this library.
    pub unsafe fn native_free(ptr: *mut u8, len: usize) {
        if !ptr.is_null() {
            drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
        }
    }
}

/// Defines a NAMU task. See the crate docs.
#[macro_export]
macro_rules! task {
    (fn $name:ident($($args:tt)*) -> $ret:ty $body:block) => {
        fn $name($($args)*) -> $ret $body
        $crate::__decoder!(__namu_decode, $name($($args)*) -> $ret);
        $crate::__export_task!();
    };
}

/// Defines a NAMU stream task: it returns an iterator, whose items are pulled one at a time.
/// See the crate docs.
#[macro_export]
macro_rules! stream {
    (fn $name:ident($($args:tt)*) -> $ret:ty $body:block) => {
        fn $name($($args)*) -> $ret $body
        $crate::__decoder!(__namu_decode, $name($($args)*) -> $ret);
        $crate::__export_stream!();
    };
}

/// Defines `$decoder(input)`: decodes the packed JSON arguments and calls the task. The input is
/// owned, so what the task returns (a stream's iterator) cannot borrow from it.
#[doc(hidden)]
#[macro_export]
macro_rules! __decoder {
    ($decoder:ident, $name:ident() -> $ret:ty) => {
        fn $decoder(input: String) -> ::std::result::Result<$ret, $crate::__private::serde_json::Error> {
            $crate::__private::serde_json::from_str::<Option<()>>(&input)?;
            Ok($name())
        }
    };
    ($decoder:ident, $name:ident($arg:ident : $ty:ty $(,)?) -> $ret:ty) => {
        fn $decoder(input: String) -> ::std::result::Result<$ret, $crate::__private::serde_json::Error> {
            let $arg: $ty = $crate::__private::serde_json::from_str(&input)?;
            Ok($name($arg))
        }
    };
    ($decoder:ident, $name:ident($($arg:ident : $ty:ty),+ $(,)?) -> $ret:ty) => {
        fn $decoder(input: String) -> ::std::result::Result<$ret, $crate::__private::serde_json::Error> {
            let ($($arg,)+): ($($ty,)+) = $crate::__private::serde_json::from_str(&input)?;
            Ok($name($($arg),+))
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __export_task {
    () => {
        fn __namu_call(input: &str) -> ::std::result::Result<String, String> {
            $crate::__private::run(input, __namu_decode)
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
        pub unsafe extern "C" fn namu_task_call(
            in_ptr: *const u8,
            in_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            unsafe { $crate::__private::native_call(in_ptr, in_len, out_ptr, out_len, __namu_call) }
        }

        $crate::__export_native_common!();
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __export_stream {
    () => {
        fn __namu_open(input: &str) -> ::std::result::Result<$crate::__private::Items, String> {
            $crate::__private::open(input, __namu_decode)
        }

        #[cfg(target_family = "wasm")]
        const _: () = {
            use $crate::stream_bindings::exports::namu::task::source::{Guest, GuestItems, Items};

            struct NamuStream;
            struct NamuItems(::std::cell::RefCell<$crate::__private::Items>);

            impl Guest for NamuStream {
                type Items = NamuItems;

                fn open(input: String) -> ::std::result::Result<Items, String> {
                    let items = __namu_open(&input)?;
                    Ok(Items::new(NamuItems(::std::cell::RefCell::new(items))))
                }
            }

            impl GuestItems for NamuItems {
                fn next(&self) -> Option<::std::result::Result<String, String>> {
                    self.0.borrow_mut().next()
                }
            }

            $crate::stream_bindings::export_stream!(NamuStream with_types_in $crate::stream_bindings);
        };

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_stream_open(
            in_ptr: *const u8,
            in_len: usize,
            handle: *mut *mut ::std::ffi::c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            unsafe {
                $crate::__private::native_open(in_ptr, in_len, handle, out_ptr, out_len, __namu_open)
            }
        }

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_stream_next(
            handle: *mut ::std::ffi::c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            unsafe { $crate::__private::native_next(handle, out_ptr, out_len) }
        }

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_stream_close(handle: *mut ::std::ffi::c_void) {
            unsafe { $crate::__private::native_close(handle) }
        }

        $crate::__export_native_common!();
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __export_native_common {
    () => {
        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub extern "C" fn namu_abi_version() -> u32 {
            $crate::NATIVE_ABI_VERSION
        }

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_task_free(ptr: *mut u8, len: usize) {
            unsafe { $crate::__private::native_free(ptr, len) }
        }
    };
}
