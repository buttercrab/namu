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
//!
//! A task's types derive [`Describe`] (or `namu::Data`, which describes too), so `namu build` can
//! record its signature, and a workflow that calls it with the wrong types is refused when it is
//! published rather than failing its runs:
//!
//! ```ignore
//! #[derive(Serialize, Deserialize, namu_guest::Describe)]
//! pub struct Stats { words: usize, top_word: String }
//! ```
//!
//! A native build of the crate exports `namu_signature`, which `namu build` reads (it builds a
//! WASM task for the host too, for that).

// The derives name this crate by its name, as tasks see it.
#[cfg(test)]
extern crate self as namu_guest;

pub use namu_core::types::Describe;
/// Describes a type's shape, as it crosses: `#[derive(Serialize, Deserialize, Describe)]`.
pub use namu_macros::GuestDescribe as Describe;

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
/// key, version 7 made a call's or a batch item's failure a [`Raised`](namu_core::raise::Raised):
/// its message and its own error value.
pub const NATIVE_ABI_VERSION: u32 = 7;

std::thread_local! {
    static CALL_KEY: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static CALL_KEYS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The key of the call this task is running for: the same each time the engine runs the call
/// again after a crash, and different for every other call. A task with side effects passes it
/// to what it calls (an idempotency key) so the effect happens once. A stream has the key of the
/// call that opened it, while it opens and while each item is pulled. `None` outside a call, and
/// in batch tasks, whose steps serve many calls at once: see [`call_keys`].
pub fn call_key() -> Option<String> {
    CALL_KEY.with_borrow(Clone::clone)
}

/// The keys of the calls a batch step serves, in the order of its items: the list a dynamic batch
/// task is given, or `admit` for a continuous one (which keeps a key with its item if it needs it
/// later). Empty outside a batch step.
pub fn call_keys() -> Vec<String> {
    CALL_KEYS.with_borrow(Clone::clone)
}

/// A batch task that keeps items across steps (continuous batching). A session is made with
/// `Default` when the worker opens it.
pub trait Continuous: Default + 'static {
    type In: serde::de::DeserializeOwned;
    type Out: serde::Serialize;
    /// What an item fails with: a message (`anyhow::Error`), or a type of its own, which an
    /// attempt of the call catches.
    type Error: namu_core::raise::Raise;

    /// Takes in `admit` and returns what finished in this step, each with its own result.
    fn step(&mut self, admit: Vec<(u64, Self::In)>) -> Stepped<Self>;
}

/// What a continuous batch's step finished: each item's id, and its output or its error.
pub type Stepped<C> = Vec<(
    u64,
    Result<<C as Continuous>::Out, <C as Continuous>::Error>,
)>;

#[doc(hidden)]
pub mod __private {
    use std::any::Any;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    pub use namu_core::raise::{Raise, Raised};
    pub use namu_core::signature::Signature;
    pub use namu_core::types::{Def, Defs, Describe, Type, named};
    pub use rmp_serde;

    /// Runs `f` with `key` as the current call's key.
    pub fn with_key<T>(key: String, f: impl FnOnce() -> T) -> T {
        lend_key(&mut Some(key), f)
    }

    /// Runs `f` with `key` lent as the current call's key, and takes it back after.
    fn lend_key<T>(key: &mut Option<String>, f: impl FnOnce() -> T) -> T {
        struct Back<'a> {
            key: &'a mut Option<String>,
            previous: Option<String>,
        }
        impl Drop for Back<'_> {
            fn drop(&mut self) {
                *self.key = super::CALL_KEY.replace(self.previous.take());
            }
        }
        let previous = super::CALL_KEY.replace(key.take());
        let _back = Back { key, previous };
        f()
    }

    /// Opens a stream for the call named `key`, which stays its key while it opens and while
    /// each item is pulled.
    pub fn open_keyed(
        key: String,
        input: &[u8],
        open: fn(&[u8]) -> Result<Items, Raised>,
    ) -> Result<Items, Raised> {
        let mut key = Some(key);
        let mut items = lend_key(&mut key, || open(input))?;
        Ok(Box::new(std::iter::from_fn(move || {
            lend_key(&mut key, || items.next())
        })))
    }

    /// A value as it crosses the task boundary: MessagePack.
    pub type Bytes = Vec<u8>;

    /// A stream's items, each encoded or the task's failure.
    pub type Items = Box<dyn Iterator<Item = Result<Bytes, Raised>>>;

    /// What a batch step finished: each item's id, and its output or its own failure.
    pub type Finished = Vec<(u64, Result<Bytes, Raised>)>;

    /// An item a batch step takes in: its id, its call's key, and its input.
    pub type Admitted = (u64, String, Bytes);

    /// An open batch session: takes in items and returns what it finished. An error fails every
    /// item in it.
    pub type Session = Box<dyn FnMut(Vec<Admitted>) -> Result<Finished, String>>;

    /// Runs `f` with `keys` as the current batch step's keys.
    fn with_keys<T>(keys: Vec<String>, f: impl FnOnce() -> T) -> T {
        struct Reset(Vec<String>);
        impl Drop for Reset {
            fn drop(&mut self) {
                super::CALL_KEYS.set(std::mem::take(&mut self.0));
            }
        }
        let _reset = Reset(super::CALL_KEYS.replace(keys));
        f()
    }

    /// A dynamic batch session over `f`: each step runs it over the items given.
    pub fn dynamic<T, R, E>(f: fn(Vec<T>) -> Vec<Result<R, E>>) -> Session
    where
        T: serde::de::DeserializeOwned + 'static,
        R: serde::Serialize + 'static,
        E: Raise + 'static,
    {
        Box::new(move |admit| {
            let mut finished = Vec::new();
            let (mut ids, mut keys, mut items) = (Vec::new(), Vec::new(), Vec::new());
            for (id, key, input) in admit {
                match rmp_serde::from_slice(&input) {
                    Ok(item) => {
                        ids.push(id);
                        keys.push(key);
                        items.push(item);
                    }
                    Err(err) => finished.push((id, Err(invalid_input(err)))),
                }
            }
            if items.is_empty() {
                return Ok(finished);
            }
            let outputs = catch_unwind(AssertUnwindSafe(|| with_keys(keys, || f(items))))
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
                    .map(|(id, output)| (id, encode_raised(output))),
            );
            Ok(finished)
        })
    }

    /// A continuous batch session of `C`.
    pub fn continuous<C: crate::Continuous>() -> Session {
        let mut session = C::default();
        Box::new(move |admit| {
            let mut finished = Vec::new();
            let (mut keys, mut items) = (Vec::new(), Vec::new());
            for (id, key, input) in admit {
                match rmp_serde::from_slice(&input) {
                    Ok(item) => {
                        keys.push(key);
                        items.push((id, item));
                    }
                    Err(err) => finished.push((id, Err(invalid_input(err)))),
                }
            }
            let done = catch_unwind(AssertUnwindSafe(|| with_keys(keys, || session.step(items))))
                .map_err(|panic| format!("task panicked: {}", panic_message(&panic)))?;
            let done = done
                .into_iter()
                .map(|(id, output)| (id, encode_raised(output)));
            finished.extend(done);
            Ok(finished)
        })
    }

    /// An output, encoded, or its failure, with its own error value if it has one.
    fn encode_raised<R: serde::Serialize, E: Raise>(output: Result<R, E>) -> Result<Bytes, Raised> {
        let output = output.map_err(Raise::raise)?;
        rmp_serde::to_vec_named(&output).map_err(|err| Raised::from(err.to_string()))
    }

    fn invalid_input(err: impl std::fmt::Display) -> Raised {
        format!("invalid task input: {err}").into()
    }

    /// Runs a task body against its encoded input and encodes its result.
    pub fn run<T, E, F>(input: &[u8], body: F) -> Result<Bytes, Raised>
    where
        T: serde::Serialize,
        E: Raise,
        F: FnOnce(Bytes) -> Result<Result<T, E>, rmp_serde::decode::Error>,
    {
        let output = catch_unwind(AssertUnwindSafe(|| body(input.to_vec())))
            .map_err(|panic| format!("task panicked: {}", panic_message(&panic)))?;
        match output {
            Ok(output) => encode_raised(output),
            Err(err) => Err(invalid_input(err)),
        }
    }

    /// Opens a stream task against its encoded input; its items are encoded as they are pulled.
    pub fn open<T, E, I, F>(input: &[u8], body: F) -> Result<Items, Raised>
    where
        T: serde::Serialize,
        E: Raise,
        I: Iterator<Item = Result<T, E>> + 'static,
        F: FnOnce(Bytes) -> Result<Result<I, E>, rmp_serde::decode::Error>,
    {
        let items = catch_unwind(AssertUnwindSafe(|| body(input.to_vec())))
            .map_err(|panic| format!("task panicked: {}", panic_message(&panic)))?;
        let mut items = match items {
            Ok(Ok(items)) => items,
            Ok(Err(err)) => return Err(err.raise()),
            Err(err) => return Err(invalid_input(err)),
        };
        Ok(Box::new(std::iter::from_fn(move || {
            let item = catch_unwind(AssertUnwindSafe(|| items.next()));
            Some(match item {
                Ok(None) => return None,
                Ok(Some(output)) => encode_raised(output),
                Err(panic) => Err(format!("task panicked: {}", panic_message(&panic)).into()),
            })
        })))
    }

    /// What a single task gives: the `T` of its `Result<T, E>`. Read off its decoder, whose type
    /// is the task's.
    pub fn single_output<F, T, E>(_decode: F, defs: &mut Defs) -> Type
    where
        F: FnOnce(Bytes) -> Result<Result<T, E>, rmp_serde::decode::Error>,
        T: Describe,
    {
        T::describe(defs)
    }

    /// What a single task fails with: the `E` of its `Result<T, E>`, if it is a type of its own.
    pub fn single_error<F, T, E>(_decode: F, defs: &mut Defs) -> Option<Type>
    where
        F: FnOnce(Bytes) -> Result<Result<T, E>, rmp_serde::decode::Error>,
        E: Raise,
    {
        E::error_type(defs)
    }

    /// What a stream gives, as a workflow sees it: the list of its items.
    pub fn stream_output<F, I, T, E, E2>(_decode: F, defs: &mut Defs) -> Type
    where
        F: FnOnce(Bytes) -> Result<Result<I, E>, rmp_serde::decode::Error>,
        I: Iterator<Item = Result<T, E2>>,
        T: Describe,
    {
        Type::List(Box::new(T::describe(defs)))
    }

    /// What a stream fails with, opening or at an item (which ends it): the `E` of its
    /// `Result<impl Iterator<Item = Result<T, E>>, E>`, if it is a type of its own.
    pub fn stream_error<F, I, T, E>(_decode: F, defs: &mut Defs) -> Option<Type>
    where
        F: FnOnce(Bytes) -> Result<Result<I, E>, rmp_serde::decode::Error>,
        I: Iterator<Item = Result<T, E>>,
        E: Raise,
    {
        E::error_type(defs)
    }

    /// A dynamic batch task's signature, per item, as a workflow calls it.
    pub fn batch_signature<T: Describe, R: Describe, E: Raise>(
        _task: fn(Vec<T>) -> Vec<Result<R, E>>,
    ) -> Signature {
        let mut defs = Defs::new();
        let input = T::describe(&mut defs);
        let output = R::describe(&mut defs);
        let error = E::error_type(&mut defs);
        Signature::new(vec![input], output, defs).failing_with(error)
    }

    /// A continuous batch task's signature, per item.
    pub fn continuous_signature<C>() -> Signature
    where
        C: crate::Continuous,
        C::In: Describe,
        C::Out: Describe,
    {
        let mut defs = Defs::new();
        let input = C::In::describe(&mut defs);
        let output = C::Out::describe(&mut defs);
        let error = C::Error::error_type(&mut defs);
        Signature::new(vec![input], output, defs).failing_with(error)
    }

    /// Hands the task's signature, as MessagePack, to `namu build`.
    ///
    /// # Safety
    /// `out_ptr` and `out_len` must be writable.
    pub unsafe fn native_signature(
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
        signature: fn() -> Signature,
    ) -> i32 {
        let (code, payload) = match rmp_serde::to_vec_named(&signature()) {
            Ok(bytes) => (0, bytes),
            Err(err) => (1, err.to_string().into_bytes()),
        };
        unsafe { hand_over(payload, out_ptr, out_len) };
        code
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

    /// Native ABI entry point shared by every native task. The key is UTF-8. A failure is handed
    /// over as MessagePack, `[message, error]`, `error` being the task's own error value or nil.
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
        call: fn(&[u8]) -> Result<Bytes, Raised>,
    ) -> i32 {
        let key = String::from_utf8_lossy(unsafe { input(key_ptr, key_len) }).into_owned();
        let (code, payload) = match with_key(key, || call(unsafe { input(in_ptr, in_len) })) {
            Ok(output) => (0, output),
            Err(raised) => (1, encode_failure(&raised)),
        };
        unsafe { hand_over(payload, out_ptr, out_len) };
        code
    }

    /// Native ABI entry point that opens a stream. On success `handle` is set; on failure the
    /// failure is handed over in `out_ptr`/`out_len`, as MessagePack `[message, error]`.
    ///
    /// # Safety
    /// `in_ptr` must point to `in_len` readable bytes; the out-pointers must be writable.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn native_open(
        key_ptr: *const u8,
        key_len: usize,
        in_ptr: *const u8,
        in_len: usize,
        handle: *mut *mut std::ffi::c_void,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
        open: fn(&[u8]) -> Result<Items, Raised>,
    ) -> i32 {
        let key = String::from_utf8_lossy(unsafe { input(key_ptr, key_len) }).into_owned();
        match open_keyed(key, unsafe { input(in_ptr, in_len) }, open) {
            Ok(items) => {
                unsafe { *handle = Box::into_raw(Box::new(items)).cast() };
                0
            }
            Err(raised) => {
                unsafe { hand_over(encode_failure(&raised), out_ptr, out_len) };
                1
            }
        }
    }

    /// Native ABI entry point for a stream's next item: 0 and the item, 1 at the end, or 2 and
    /// the task's failure, as MessagePack `[message, error]`.
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
            Some(Err(raised)) => (2, encode_failure(&raised)),
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

    /// An item of a batch step as it crosses the native ABI: its id, its key and its encoded
    /// input.
    type NativeAdmitted = (u64, String, serde_bytes::ByteBuf);

    /// An item a batch step finished: its id and its encoded output, or its failure.
    type Done = (u64, Result<serde_bytes::ByteBuf, Raised>);

    /// A failure as it crosses the native ABI: MessagePack, `[message, error]`.
    fn encode_failure(raised: &Raised) -> Bytes {
        rmp_serde::to_vec(raised).unwrap_or_else(|_| {
            let message = Raised::from(raised.message.clone());
            rmp_serde::to_vec(&message).expect("a message encodes")
        })
    }

    /// Native ABI entry point for a batch step. The input is a MessagePack array of
    /// `[id, key, input]`, the output one of `[id, {"Ok": output} | {"Err": [message, error]}]`;
    /// or 1 and the session's error.
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
        let step = rmp_serde::from_slice::<Vec<NativeAdmitted>>(unsafe { input(in_ptr, in_len) })
            .map_err(|err| err.to_string())
            .map(|admit| {
                admit
                    .into_iter()
                    .map(|(id, key, input)| (id, key, input.into_vec()))
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
        $crate::__signature!(single_output, single_error, $name($($args)*));
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
        $crate::__signature!(stream_output, stream_error, $name($($args)*));
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
        fn __namu_signature() -> $crate::__private::Signature {
            $crate::__private::batch_signature($name)
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
        fn __namu_signature() -> $crate::__private::Signature {
            $crate::__private::continuous_signature::<$session>()
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

/// Defines `__namu_signature()`: the task's parameters, described, and its output and error,
/// read off its decoder by `$output` and `$error`.
#[doc(hidden)]
#[macro_export]
macro_rules! __signature {
    ($output:ident, $error:ident, $name:ident($($arg:ident : $ty:ty),* $(,)?)) => {
        fn __namu_signature() -> $crate::__private::Signature {
            let mut defs = $crate::__private::Defs::new();
            let params = ::std::vec![
                $(<$ty as $crate::__private::Describe>::describe(&mut defs)),*
            ];
            let output = $crate::__private::$output(__namu_decode, &mut defs);
            let error = $crate::__private::$error(__namu_decode, &mut defs);
            $crate::__private::Signature::new(params, output, defs).failing_with(error)
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __export_task {
    () => {
        fn __namu_call(
            input: &[u8],
        ) -> ::std::result::Result<$crate::__private::Bytes, $crate::__private::Raised> {
            $crate::__private::run(input, __namu_decode)
        }

        #[cfg(target_family = "wasm")]
        const _: () = {
            use $crate::bindings::Failure;

            struct NamuTask;
            impl $crate::bindings::Guest for NamuTask {
                fn call(key: String, input: Vec<u8>) -> ::std::result::Result<Vec<u8>, Failure> {
                    $crate::__private::with_key(key, || __namu_call(&input)).map_err(|raised| {
                        Failure {
                            message: raised.message,
                            error: raised.error,
                        }
                    })
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
        fn __namu_open(
            input: &[u8],
        ) -> ::std::result::Result<$crate::__private::Items, $crate::__private::Raised> {
            $crate::__private::open(input, __namu_decode)
        }

        #[cfg(target_family = "wasm")]
        const _: () = {
            use $crate::stream_bindings::exports::namu::task::source::{
                Failure, Guest, GuestItems, Items,
            };

            fn failure(raised: $crate::__private::Raised) -> Failure {
                Failure {
                    message: raised.message,
                    error: raised.error,
                }
            }

            struct NamuStream;
            struct NamuItems(::std::cell::RefCell<$crate::__private::Items>);

            impl Guest for NamuStream {
                type Items = NamuItems;

                fn open(key: String, input: Vec<u8>) -> ::std::result::Result<Items, Failure> {
                    let items =
                        $crate::__private::open_keyed(key, &input, __namu_open).map_err(failure)?;
                    Ok(Items::new(NamuItems(::std::cell::RefCell::new(items))))
                }
            }

            impl GuestItems for NamuItems {
                fn next(&self) -> Option<::std::result::Result<Vec<u8>, Failure>> {
                    self.0.borrow_mut().next().map(|item| item.map_err(failure))
                }
            }

            $crate::stream_bindings::export_stream!(NamuStream with_types_in $crate::stream_bindings);
        };

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_stream_open(
            key_ptr: *const u8,
            key_len: usize,
            in_ptr: *const u8,
            in_len: usize,
            handle: *mut *mut ::std::ffi::c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            unsafe {
                $crate::__private::native_open(
                    key_ptr,
                    key_len,
                    in_ptr,
                    in_len,
                    handle,
                    out_ptr,
                    out_len,
                    __namu_open,
                )
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
                Admitted, Failure, Finished, Guest, GuestSession, Session,
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
                        .map(|item| (item.id, item.key, item.input))
                        .collect();
                    let finished = (self.0.borrow_mut())(admit)?;
                    Ok(finished
                        .into_iter()
                        .map(|(id, output)| Finished {
                            id,
                            output: output.map_err(|raised| Failure {
                                message: raised.message,
                                error: raised.error,
                            }),
                        })
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

        /// The task's signature, as MessagePack, for `namu build`.
        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_signature(out_ptr: *mut *mut u8, out_len: *mut usize) -> i32 {
            unsafe { $crate::__private::native_signature(out_ptr, out_len, __namu_signature) }
        }

        #[cfg(target_family = "wasm")]
        const _: fn() -> $crate::__private::Signature = __namu_signature;

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn namu_task_free(ptr: *mut u8, len: usize) {
            unsafe { $crate::__private::native_free(ptr, len) }
        }
    };
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::__private::{
        Bytes, Items, Raised, dynamic, native_call, native_free, open_keyed, rmp_serde, run,
    };
    use super::{call_key, call_keys};

    fn whoami(_input: &[u8]) -> Result<Bytes, Raised> {
        call_key()
            .map(String::into_bytes)
            .ok_or_else(|| "no key".to_string().into())
    }

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize, super::Describe)]
    enum Fetch {
        NotFound,
        Limited { retry_after: u64 },
    }

    impl std::fmt::Display for Fetch {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{self:?}")
        }
    }

    fn limited(_input: Bytes) -> Result<Result<(), Fetch>, rmp_serde::decode::Error> {
        Ok(Err(Fetch::Limited { retry_after: 3 }))
    }

    fn failed(_input: Bytes) -> Result<Result<(), anyhow::Error>, rmp_serde::decode::Error> {
        Ok(Err(anyhow::anyhow!("no luck")))
    }

    #[test]
    fn a_task_s_own_error_crosses_with_its_message() {
        let raised = run(&[0xc0], limited).unwrap_err();
        assert_eq!(raised.message, "Limited { retry_after: 3 }");
        let error: Fetch = rmp_serde::from_slice(&raised.error.unwrap()).unwrap();
        assert_eq!(error, Fetch::Limited { retry_after: 3 });
        // A message is only a message.
        let raised = run(&[0xc0], failed).unwrap_err();
        assert_eq!((raised.message.as_str(), raised.error), ("no luck", None));
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

    fn keys(_input: &[u8]) -> Result<Items, Raised> {
        let opened = call_key();
        Ok(Box::new([opened, None].into_iter().map(|key| {
            Ok(key.or_else(call_key).unwrap_or_default().into_bytes())
        })))
    }

    #[test]
    fn a_stream_has_its_key_when_it_opens_and_at_every_item() {
        let mut items = open_keyed("k".into(), &[], keys).unwrap();
        assert_eq!(call_key(), None);
        // The first item is the key seen at open, the second the one seen when pulled.
        assert_eq!(items.next(), Some(Ok(b"k".to_vec())));
        assert_eq!(items.next(), Some(Ok(b"k".to_vec())));
        assert_eq!(call_key(), None);
    }

    #[test]
    fn a_batch_step_has_the_keys_of_its_items_in_order() {
        let mut session = dynamic(|xs: Vec<i32>| {
            let keys = call_keys();
            xs.into_iter()
                .zip(keys)
                .map(|(x, key)| Ok::<_, String>(format!("{x}:{key}")))
                .collect()
        });
        let item = |id, key: &str, x: i32| (id, key.to_string(), rmp_serde::to_vec(&x).unwrap());
        // An item whose input does not decode is left out of the list, and of the keys.
        let bad = (2, "b".to_string(), vec![0xc1]);
        let finished = session(vec![item(1, "a", 10), bad, item(3, "c", 30)]).unwrap();
        let outputs: Vec<_> = finished
            .into_iter()
            .filter_map(|(id, output)| {
                Some((id, rmp_serde::from_slice::<String>(&output.ok()?).ok()?))
            })
            .collect();
        assert_eq!(outputs, [(1, "10:a".to_string()), (3, "30:c".to_string())]);
        assert!(call_keys().is_empty());
    }
}
