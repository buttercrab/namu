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
//!
//! // Dynamic batching: each step runs the function over the items it is given.
//! namu_guest::batch! {
//!     fn embed(texts: Vec<String>) -> Vec<anyhow::Result<Embedding>> {
//!         model().embed(&texts)
//!     }
//! }
//!
//! // Continuous batching: the session keeps items across steps.
//! namu_guest::continuous!(Decoder); // Decoder: namu_guest::Continuous + Default
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
pub mod batch_bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "batch-task",
        pub_export_macro: true,
        export_macro_name: "export_batch",
        default_bindings_module: "::namu_guest::batch_bindings",
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

/// Version of the native C ABI exported by the task macros. Version 3 added streams, version 4
/// batch sessions, version 5 moved values from JSON to MessagePack, version 6 gave each call its
/// key.
pub const NATIVE_ABI_VERSION: u32 = 6;

std::thread_local! {
    static CALL_KEY: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// The key of the call this task is running for: the same each time the engine runs the call
/// again after a crash, and different for every other call. A task with side effects passes it
/// to what it calls (an idempotency key) so the effect happens once. `None` outside a call, and
/// in stream and batch tasks.
pub fn call_key() -> Option<String> {
    CALL_KEY.with_borrow(Clone::clone)
}

/// A batch task that keeps items across steps (continuous batching). A session is made with
/// `Default` when the worker opens it.
pub trait Continuous: Default + 'static {
    type In: serde::de::DeserializeOwned;
    type Out: serde::Serialize;

    /// Takes in `admit` and returns what finished in this step, each with its own result.
    fn step(&mut self, admit: Vec<(u64, Self::In)>) -> Vec<(u64, Result<Self::Out, String>)>;
}

#[doc(hidden)]
pub mod __private {
    use std::any::Any;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    pub use rmp_serde;

    /// Runs `f` with `key` as the current call's key.
    pub fn with_key<T>(key: String, f: impl FnOnce() -> T) -> T {
        struct Reset(Option<String>);
        impl Drop for Reset {
            fn drop(&mut self) {
                super::CALL_KEY.set(self.0.take());
            }
        }
        let _reset = Reset(super::CALL_KEY.replace(Some(key)));
        f()
    }

    /// A value as it crosses the task boundary: MessagePack.
    pub type Bytes = Vec<u8>;

    /// A stream's items, each encoded or the task's error.
    pub type Items = Box<dyn Iterator<Item = Result<Bytes, String>>>;

    /// What a batch step finished: each item's id, and its output or its own error.
    pub type Finished = Vec<(u64, Result<Bytes, String>)>;

    /// An open batch session: takes in `(id, input)` and returns what it finished. An error fails
    /// every item in it.
    pub type Session = Box<dyn FnMut(Vec<(u64, Bytes)>) -> Result<Finished, String>>;

    /// A dynamic batch session over `f`: each step runs it over the items given.
    pub fn dynamic<T, R, E>(f: fn(Vec<T>) -> Vec<Result<R, E>>) -> Session
    where
        T: serde::de::DeserializeOwned + 'static,
        R: serde::Serialize + 'static,
        E: std::fmt::Display + 'static,
    {
        Box::new(move |admit| {
            let mut finished = Vec::new();
            let (mut ids, mut items) = (Vec::new(), Vec::new());
            for (id, input) in admit {
                match rmp_serde::from_slice(&input) {
                    Ok(item) => {
                        ids.push(id);
                        items.push(item);
                    }
                    Err(err) => finished.push((id, Err(format!("invalid task input: {err}")))),
                }
            }
            if items.is_empty() {
                return Ok(finished);
            }
            let outputs = catch_unwind(AssertUnwindSafe(|| f(items)))
                .map_err(|panic| format!("task panicked: {}", panic_message(&panic)))?;
            if outputs.len() != ids.len() {
                return Err(format!(
                    "a batch of {} gave {} results",
                    ids.len(),
                    outputs.len()
                ));
            }
            finished.extend(
                ids.into_iter()
                    .zip(outputs)
                    .map(|(id, output)| (id, encode(output))),
            );
            Ok(finished)
        })
    }

    /// A continuous batch session of `C`.
    pub fn continuous<C: crate::Continuous>() -> Session {
        let mut session = C::default();
        Box::new(move |admit| {
            let mut finished = Vec::new();
            let mut items = Vec::new();
            for (id, input) in admit {
                match rmp_serde::from_slice(&input) {
                    Ok(item) => items.push((id, item)),
                    Err(err) => finished.push((id, Err(format!("invalid task input: {err}")))),
                }
            }
            let done = catch_unwind(AssertUnwindSafe(|| session.step(items)))
                .map_err(|panic| format!("task panicked: {}", panic_message(&panic)))?;
            finished.extend(done.into_iter().map(|(id, output)| (id, encode(output))));
            Ok(finished)
        })
    }

    fn encode<R: serde::Serialize, E: std::fmt::Display>(
        output: Result<R, E>,
    ) -> Result<Bytes, String> {
        output
            .map_err(|err| err.to_string())
            .and_then(|output| rmp_serde::to_vec_named(&output).map_err(|err| err.to_string()))
    }

    /// Runs a task body against its encoded input and encodes its result.
    pub fn run<T, E, F>(input: &[u8], body: F) -> Result<Bytes, String>
    where
        T: serde::Serialize,
        E: std::fmt::Display,
        F: FnOnce(Bytes) -> Result<Result<T, E>, rmp_serde::decode::Error>,
    {
        let output = catch_unwind(AssertUnwindSafe(|| body(input.to_vec())))
            .map_err(|panic| format!("task panicked: {}", panic_message(&panic)))?;
        match output {
            Ok(output) => encode(output),
            Err(err) => Err(format!("invalid task input: {err}")),
        }
    }

    /// Opens a stream task against its encoded input; its items are encoded as they are pulled.
    pub fn open<T, E, I, F>(input: &[u8], body: F) -> Result<Items, String>
    where
        T: serde::Serialize,
        E: std::fmt::Display,
        I: Iterator<Item = Result<T, E>> + 'static,
        F: FnOnce(Bytes) -> Result<Result<I, E>, rmp_serde::decode::Error>,
    {
        let items = catch_unwind(AssertUnwindSafe(|| body(input.to_vec())))
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
                Ok(Some(output)) => encode(output),
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
    unsafe fn hand_over(payload: Bytes, out_ptr: *mut *mut u8, out_len: *mut usize) {
        let mut bytes = payload.into_boxed_slice();
        unsafe {
            *out_len = bytes.len();
            *out_ptr = bytes.as_mut_ptr();
        }
        std::mem::forget(bytes);
    }

    /// # Safety
    /// `in_ptr` must point to `in_len` readable bytes.
    unsafe fn input<'a>(in_ptr: *const u8, in_len: usize) -> &'a [u8] {
        unsafe { std::slice::from_raw_parts(in_ptr, in_len) }
    }

    /// Native ABI entry point shared by every native task. The key is UTF-8.
    ///
    /// # Safety
    /// `key_ptr` must point to `key_len` readable bytes and `in_ptr` to `in_len`; `out_ptr` and
    /// `out_len` must be writable.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn native_call(
        key_ptr: *const u8,
        key_len: usize,
        in_ptr: *const u8,
        in_len: usize,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
        call: fn(&[u8]) -> Result<Bytes, String>,
    ) -> i32 {
        let key = String::from_utf8_lossy(unsafe { input(key_ptr, key_len) }).into_owned();
        let (code, payload) = match with_key(key, || call(unsafe { input(in_ptr, in_len) })) {
            Ok(output) => (0, output),
            Err(err) => (1, err.into_bytes()),
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
        open: fn(&[u8]) -> Result<Items, String>,
    ) -> i32 {
        match open(unsafe { input(in_ptr, in_len) }) {
            Ok(items) => {
                unsafe { *handle = Box::into_raw(Box::new(items)).cast() };
                0
            }
            Err(err) => {
                unsafe { hand_over(err.into_bytes(), out_ptr, out_len) };
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
            Some(Err(err)) => (2, err.into_bytes()),
        };
        unsafe { hand_over(payload, out_ptr, out_len) };
        code
    }

    /// # Safety
    /// `handle` must come from [`native_open`] and not be closed yet.
    pub unsafe fn native_close(handle: *mut std::ffi::c_void) {
        drop(unsafe { Box::from_raw(handle.cast::<Items>()) });
    }

    /// Native ABI entry point that opens a batch session; on failure the error is handed over.
    ///
    /// # Safety
    /// The out-pointers must be writable.
    pub unsafe fn native_batch_open(
        handle: *mut *mut std::ffi::c_void,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
        open: fn() -> Result<Session, String>,
    ) -> i32 {
        match catch_unwind(open)
            .unwrap_or_else(|panic| Err(format!("task panicked: {}", panic_message(&panic))))
        {
            Ok(session) => {
                unsafe { *handle = Box::into_raw(Box::new(session)).cast() };
                0
            }
            Err(err) => {
                unsafe { hand_over(err.into_bytes(), out_ptr, out_len) };
                1
            }
        }
    }

    /// An item of a batch step as it crosses the native ABI: its id and its encoded input.
    type Admitted = (u64, serde_bytes::ByteBuf);

    /// An item a batch step finished: its id and its encoded output, or its error.
    type Done = (u64, Result<serde_bytes::ByteBuf, String>);

    /// Native ABI entry point for a batch step. The input is a MessagePack array of `[id, input]`,
    /// the output one of `[id, {"Ok": output} | {"Err": error}]`; or 1 and the session's error.
    ///
    /// # Safety
    /// `handle` must come from [`native_batch_open`] and not be closed; `in_ptr` must point to
    /// `in_len` readable bytes; the out-pointers must be writable.
    pub unsafe fn native_batch_step(
        handle: *mut std::ffi::c_void,
        in_ptr: *const u8,
        in_len: usize,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
    ) -> i32 {
        let session = unsafe { &mut *handle.cast::<Session>() };
        let step = rmp_serde::from_slice::<Vec<Admitted>>(unsafe { input(in_ptr, in_len) })
            .map_err(|err| err.to_string())
            .map(|admit| {
                admit
                    .into_iter()
                    .map(|(id, input)| (id, input.into_vec()))
                    .collect()
            })
            .and_then(session)
            .and_then(|finished| {
                let finished: Vec<Done> = finished
                    .into_iter()
                    .map(|(id, output)| (id, output.map(serde_bytes::ByteBuf::from)))
                    .collect();
                rmp_serde::to_vec(&finished).map_err(|err| err.to_string())
            });
        let (code, payload) = match step {
            Ok(finished) => (0, finished),
            Err(err) => (1, err.into_bytes()),
        };
        unsafe { hand_over(payload, out_ptr, out_len) };
        code
    }

    /// # Safety
    /// `handle` must come from [`native_batch_open`] and not be closed yet.
    pub unsafe fn native_batch_close(handle: *mut std::ffi::c_void) {
        drop(unsafe { Box::from_raw(handle.cast::<Session>()) });
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

/// Defines a NAMU batch task run by dynamic batching: the function takes the items of a step and
/// returns one result per item. See the crate docs.
#[macro_export]
macro_rules! batch {
    (fn $name:ident($arg:ident : $ty:ty $(,)?) -> $ret:ty $body:block) => {
        fn $name($arg: $ty) -> $ret $body
        fn __namu_open_batch() -> ::std::result::Result<$crate::__private::Session, String> {
            Ok($crate::__private::dynamic($name))
        }
        $crate::__export_batch!();
    };
}

/// Defines a NAMU batch task run by continuous batching, from a type implementing
/// [`Continuous`]. See the crate docs.
#[macro_export]
macro_rules! continuous {
    ($session:ty) => {
        fn __namu_open_batch() -> ::std::result::Result<$crate::__private::Session, String> {
            Ok($crate::__private::continuous::<$session>())
        }
        $crate::__export_batch!();
    };
}

/// Defines `$decoder(input)`: decodes the packed arguments and calls the task. The input is
/// owned, so what the task returns (a stream's iterator) cannot borrow from it.
#[doc(hidden)]
#[macro_export]
macro_rules! __decoder {
    ($decoder:ident, $name:ident() -> $ret:ty) => {
        fn $decoder(
            input: $crate::__private::Bytes,
        ) -> ::std::result::Result<$ret, $crate::__private::rmp_serde::decode::Error> {
            $crate::__private::rmp_serde::from_slice::<()>(&input)?;
            Ok($name())
        }
    };
    ($decoder:ident, $name:ident($arg:ident : $ty:ty $(,)?) -> $ret:ty) => {
        fn $decoder(
            input: $crate::__private::Bytes,
        ) -> ::std::result::Result<$ret, $crate::__private::rmp_serde::decode::Error> {
            let $arg: $ty = $crate::__private::rmp_serde::from_slice(&input)?;
            Ok($name($arg))
        }
    };
    ($decoder:ident, $name:ident($($arg:ident : $ty:ty),+ $(,)?) -> $ret:ty) => {
        fn $decoder(
            input: $crate::__private::Bytes,
        ) -> ::std::result::Result<$ret, $crate::__private::rmp_serde::decode::Error> {
            let ($($arg,)+): ($($ty,)+) = $crate::__private::rmp_serde::from_slice(&input)?;
            Ok($name($($arg),+))
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __export_task {
    () => {
        fn __namu_call(input: &[u8]) -> ::std::result::Result<$crate::__private::Bytes, String> {
            $crate::__private::run(input, __namu_decode)
        }

        #[cfg(target_family = "wasm")]
        const _: () = {
            struct NamuTask;
            impl $crate::bindings::Guest for NamuTask {
                fn call(key: String, input: Vec<u8>) -> ::std::result::Result<Vec<u8>, String> {
                    $crate::__private::with_key(key, || __namu_call(&input))
                }
            }
    $crate::bindings::export!(NamuTask with_types_in $crate::bindings);
        };

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_task_call(
            key_ptr: *const u8,
            key_len: usize,
            in_ptr: *const u8,
            in_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            unsafe {
                $crate::__private::native_call(
                    key_ptr,
                    key_len,
                    in_ptr,
                    in_len,
                    out_ptr,
                    out_len,
                    __namu_call,
                )
            }
        }

        $crate::__export_native_common!();
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __export_stream {
    () => {
        fn __namu_open(input: &[u8]) -> ::std::result::Result<$crate::__private::Items, String> {
            $crate::__private::open(input, __namu_decode)
        }

        #[cfg(target_family = "wasm")]
        const _: () = {
            use $crate::stream_bindings::exports::namu::task::source::{Guest, GuestItems, Items};

            struct NamuStream;
            struct NamuItems(::std::cell::RefCell<$crate::__private::Items>);

            impl Guest for NamuStream {
                type Items = NamuItems;

                fn open(input: Vec<u8>) -> ::std::result::Result<Items, String> {
                    let items = __namu_open(&input)?;
                    Ok(Items::new(NamuItems(::std::cell::RefCell::new(items))))
                }
            }

            impl GuestItems for NamuItems {
                fn next(&self) -> Option<::std::result::Result<Vec<u8>, String>> {
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
macro_rules! __export_batch {
    () => {
        #[cfg(target_family = "wasm")]
        const _: () = {
            use $crate::batch_bindings::exports::namu::task::batch::{
                Admitted, Finished, Guest, GuestSession, Session,
            };

            struct NamuBatch;
            struct NamuSession(::std::cell::RefCell<$crate::__private::Session>);

            impl Guest for NamuBatch {
                type Session = NamuSession;

                fn open() -> ::std::result::Result<Session, String> {
                    let session = __namu_open_batch()?;
                    Ok(Session::new(NamuSession(::std::cell::RefCell::new(
                        session,
                    ))))
                }
            }

            impl GuestSession for NamuSession {
                fn step(
                    &self,
                    admit: Vec<Admitted>,
                ) -> ::std::result::Result<Vec<Finished>, String> {
                    let admit = admit
                        .into_iter()
                        .map(|item| (item.id, item.input))
                        .collect();
                    let finished = (self.0.borrow_mut())(admit)?;
                    Ok(finished
                        .into_iter()
                        .map(|(id, output)| Finished { id, output })
                        .collect())
                }
            }

    $crate::batch_bindings::export_batch!(NamuBatch with_types_in $crate::batch_bindings);
        };

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_batch_open(
            handle: *mut *mut ::std::ffi::c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            unsafe {
                $crate::__private::native_batch_open(handle, out_ptr, out_len, __namu_open_batch)
            }
        }

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_batch_step(
            handle: *mut ::std::ffi::c_void,
            in_ptr: *const u8,
            in_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            unsafe {
                $crate::__private::native_batch_step(handle, in_ptr, in_len, out_ptr, out_len)
            }
        }

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_batch_close(handle: *mut ::std::ffi::c_void) {
            unsafe { $crate::__private::native_batch_close(handle) }
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

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::__private::{Bytes, native_call, native_free};
    use super::call_key;

    fn whoami(_input: &[u8]) -> Result<Bytes, String> {
        call_key().map(String::into_bytes).ok_or("no key".into())
    }

    #[test]
    fn a_native_call_reads_its_key() {
        let (key, input) = ("run/1.2/3/4", [0xc0u8]);
        let (mut out_ptr, mut out_len) = (std::ptr::null_mut(), 0);
        // SAFETY: valid buffers; the output is handed back to `native_free`.
        let code = unsafe {
            native_call(
                key.as_ptr(),
                key.len(),
                input.as_ptr(),
                input.len(),
                &mut out_ptr,
                &mut out_len,
                whoami,
            )
        };
        let output = unsafe { std::slice::from_raw_parts(out_ptr, out_len) }.to_vec();
        unsafe { native_free(out_ptr, out_len) };
        assert_eq!((code, output.as_slice()), (0, key.as_bytes()));
        // Only while the call runs.
        assert_eq!(call_key(), None);
    }
}
