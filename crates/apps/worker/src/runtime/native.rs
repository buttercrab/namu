//! Runs tasks compiled as native shared libraries exporting the NAMU C ABI (version 4). Every
//! buffer the library hands over is owned by it and given back through `namu_task_free`.
//!
//! - A task exports `namu_task_call`, and runs exactly once per call.
//! - A stream task exports `namu_stream_open`, `namu_stream_next` and `namu_stream_close`.
//! - A batch task exports `namu_batch_open`, `namu_batch_step` and `namu_batch_close`; a step's
//!   items and results cross as JSON.

use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;

use libloading::Library;
use namu_engine::tasks::{
    BatchSession, BatchTask, CallTask, Finished, StreamItem, StreamTask, TaskStream,
};
use serde_json::Value;

type AbiFn = unsafe extern "C" fn() -> u32;
type FreeFn = unsafe extern "C" fn(*mut u8, usize);
type CallFn = unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32;
type OpenFn =
    unsafe extern "C" fn(*const u8, usize, *mut *mut c_void, *mut *mut u8, *mut usize) -> i32;
type NextFn = unsafe extern "C" fn(*mut c_void, *mut *mut u8, *mut usize) -> i32;
type CloseFn = unsafe extern "C" fn(*mut c_void);
type BatchOpenFn = unsafe extern "C" fn(*mut *mut c_void, *mut *mut u8, *mut usize) -> i32;
type StepFn = unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut *mut u8, *mut usize) -> i32;

const ABI_VERSION: u32 = 4;

pub struct NativeTask {
    call: CallFn,
    free: FreeFn,
    // Kept loaded for the worker's lifetime: unloading native code is not reliably safe.
    _lib: Library,
}

pub struct NativeStreamTask {
    open: OpenFn,
    next: NextFn,
    close: CloseFn,
    free: FreeFn,
    _lib: Library,
}

/// Loads a library and checks its ABI version.
fn load(path: &Path) -> anyhow::Result<(Library, FreeFn)> {
    // SAFETY: task code is trusted; the symbols are checked against the expected ABI version.
    unsafe {
        let lib = Library::new(path)?;
        let version = (*lib.get::<AbiFn>(b"namu_abi_version")?)();
        anyhow::ensure!(
            version == ABI_VERSION,
            "native task ABI version {version}, expected {ABI_VERSION}; rebuild the task"
        );
        let free = *lib.get::<FreeFn>(b"namu_task_free")?;
        Ok((lib, free))
    }
}

/// Copies a buffer the library handed over, then gives it back.
fn take(free: FreeFn, ptr: *mut u8, len: usize) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: the library wrote `len` bytes at `ptr`, which we free exactly once.
    let text =
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(ptr, len) }).into_owned();
    unsafe { free(ptr, len) };
    text
}

impl NativeTask {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let (lib, free) = load(path)?;
        // SAFETY: the symbol has the ABI's signature.
        let call = unsafe { *lib.get::<CallFn>(b"namu_task_call")? };
        Ok(Self {
            call,
            free,
            _lib: lib,
        })
    }
}

impl CallTask for NativeTask {
    fn call(&self, input: Value) -> anyhow::Result<Result<Value, String>> {
        let input = input.to_string();
        let (mut out_ptr, mut out_len) = (std::ptr::null_mut(), 0);
        // SAFETY: the ABI contract: the library writes an owned buffer, which `take` frees.
        let code = unsafe { (self.call)(input.as_ptr(), input.len(), &mut out_ptr, &mut out_len) };
        let output = take(self.free, out_ptr, out_len);
        super::output(if code == 0 { Ok(output) } else { Err(output) })
    }
}

impl NativeStreamTask {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let (lib, free) = load(path)?;
        // SAFETY: the symbols have the ABI's signatures.
        unsafe {
            Ok(Self {
                open: *lib.get::<OpenFn>(b"namu_stream_open")?,
                next: *lib.get::<NextFn>(b"namu_stream_next")?,
                close: *lib.get::<CloseFn>(b"namu_stream_close")?,
                free,
                _lib: lib,
            })
        }
    }
}

impl StreamTask for NativeStreamTask {
    fn open(
        self: Arc<Self>,
        input: Value,
        _cursor: Option<String>,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>> {
        let input = input.to_string();
        let mut handle = std::ptr::null_mut();
        let (mut out_ptr, mut out_len) = (std::ptr::null_mut(), 0);
        // SAFETY: the ABI contract: a handle on success, an owned error buffer otherwise.
        let code = unsafe {
            (self.open)(
                input.as_ptr(),
                input.len(),
                &mut handle,
                &mut out_ptr,
                &mut out_len,
            )
        };
        if code != 0 {
            return Ok(Err(take(self.free, out_ptr, out_len)));
        }
        Ok(Ok(Box::new(NativeItems { task: self, handle })))
    }
}

struct NativeItems {
    task: Arc<NativeStreamTask>,
    handle: *mut c_void,
}

// SAFETY: the handle is only used through `&mut self`, from one thread at a time.
unsafe impl Send for NativeItems {}

impl TaskStream for NativeItems {
    fn next(&mut self) -> anyhow::Result<Option<Result<StreamItem, String>>> {
        let (mut out_ptr, mut out_len) = (std::ptr::null_mut(), 0);
        // SAFETY: the handle is open; the library writes an owned buffer unless the stream ended.
        let code = unsafe { (self.task.next)(self.handle, &mut out_ptr, &mut out_len) };
        Ok(match code {
            1 => None,
            0 => {
                let item = take(self.task.free, out_ptr, out_len);
                Some(
                    serde_json::from_str(&item)
                        .map_err(|err| format!("bad item: {err}"))
                        .map(|value| StreamItem {
                            value,
                            cursor: None,
                        }),
                )
            }
            _ => Some(Err(take(self.task.free, out_ptr, out_len))),
        })
    }
}

impl Drop for NativeItems {
    fn drop(&mut self) {
        // SAFETY: the handle is open and closed exactly once.
        unsafe { (self.task.close)(self.handle) };
    }
}

pub struct NativeBatchTask {
    open: BatchOpenFn,
    step: StepFn,
    close: CloseFn,
    free: FreeFn,
    _lib: Library,
}

impl NativeBatchTask {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let (lib, free) = load(path)?;
        // SAFETY: the symbols have the ABI's signatures.
        unsafe {
            Ok(Self {
                open: *lib.get::<BatchOpenFn>(b"namu_batch_open")?,
                step: *lib.get::<StepFn>(b"namu_batch_step")?,
                close: *lib.get::<CloseFn>(b"namu_batch_close")?,
                free,
                _lib: lib,
            })
        }
    }
}

impl BatchTask for NativeBatchTask {
    fn open(self: Arc<Self>) -> anyhow::Result<Result<Box<dyn BatchSession>, String>> {
        let mut handle = std::ptr::null_mut();
        let (mut out_ptr, mut out_len) = (std::ptr::null_mut(), 0);
        // SAFETY: the ABI contract: a handle on success, an owned error buffer otherwise.
        let code = unsafe { (self.open)(&mut handle, &mut out_ptr, &mut out_len) };
        if code != 0 {
            return Ok(Err(take(self.free, out_ptr, out_len)));
        }
        Ok(Ok(Box::new(NativeSession { task: self, handle })))
    }
}

struct NativeSession {
    task: Arc<NativeBatchTask>,
    handle: *mut c_void,
}

// SAFETY: the handle is only used through `&mut self`, from one thread at a time.
unsafe impl Send for NativeSession {}

impl BatchSession for NativeSession {
    fn step(&mut self, admit: Vec<(u64, Value)>) -> anyhow::Result<Finished> {
        let admit: Vec<(u64, String)> = admit
            .into_iter()
            .map(|(id, input)| (id, input.to_string()))
            .collect();
        let input = serde_json::to_string(&admit)?;
        let (mut out_ptr, mut out_len) = (std::ptr::null_mut(), 0);
        // SAFETY: the session is open; the library writes an owned buffer.
        let code = unsafe {
            (self.task.step)(
                self.handle,
                input.as_ptr(),
                input.len(),
                &mut out_ptr,
                &mut out_len,
            )
        };
        let output = take(self.task.free, out_ptr, out_len);
        anyhow::ensure!(code == 0, "{output}");
        let finished: Vec<(u64, Result<String, String>)> = serde_json::from_str(&output)?;
        Ok(finished
            .into_iter()
            .map(|(id, output)| {
                let output = output.and_then(|output| {
                    serde_json::from_str(&output).map_err(|err| format!("bad output: {err}"))
                });
                (id, output)
            })
            .collect())
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        // SAFETY: the session is open and closed exactly once.
        unsafe { (self.task.close)(self.handle) };
    }
}
