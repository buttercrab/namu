//! Runs tasks compiled as native shared libraries exporting the NAMU C ABI (version 5). Every
//! buffer the library hands over is owned by it and given back through `namu_task_free`.
//!
//! - A task exports `namu_task_call`, and runs exactly once per call.
//! - A stream task exports `namu_stream_open`, `namu_stream_next` and `namu_stream_close`.
//! - A batch task exports `namu_batch_open`, `namu_batch_step` and `namu_batch_close`; a step's
//!   items and results cross as one MessagePack document.
//!
//! Values cross as MessagePack; errors as text.

use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;

use libloading::Library;
use namu_core::value::{Value, encode_args};
use namu_engine::tasks::{
    Args, BatchSession, BatchTask, CallTask, Finished, StreamItem, StreamTask, TaskStream,
};
use serde_bytes::ByteBuf;

type AbiFn = unsafe extern "C" fn() -> u32;
type FreeFn = unsafe extern "C" fn(*mut u8, usize);
type CallFn = unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32;
type OpenFn =
    unsafe extern "C" fn(*const u8, usize, *mut *mut c_void, *mut *mut u8, *mut usize) -> i32;
type NextFn = unsafe extern "C" fn(*mut c_void, *mut *mut u8, *mut usize) -> i32;
type CloseFn = unsafe extern "C" fn(*mut c_void);
type BatchOpenFn = unsafe extern "C" fn(*mut *mut c_void, *mut *mut u8, *mut usize) -> i32;
type StepFn = unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut *mut u8, *mut usize) -> i32;

const ABI_VERSION: u32 = 5;

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
fn take(free: FreeFn, ptr: *mut u8, len: usize) -> Vec<u8> {
    if ptr.is_null() {
        return Vec::new();
    }
    // SAFETY: the library wrote `len` bytes at `ptr`, which we free exactly once.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
    unsafe { free(ptr, len) };
    bytes
}

/// An error the library handed over, as text.
fn take_error(free: FreeFn, ptr: *mut u8, len: usize) -> String {
    String::from_utf8_lossy(&take(free, ptr, len)).into_owned()
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
    fn call(&self, args: Args) -> anyhow::Result<Result<Value, String>> {
        let input = encode_args(&args)?;
        let (mut out_ptr, mut out_len) = (std::ptr::null_mut(), 0);
        // SAFETY: the ABI contract: the library writes an owned buffer, which `take` frees.
        let code = unsafe { (self.call)(input.as_ptr(), input.len(), &mut out_ptr, &mut out_len) };
        Ok(super::output(match code {
            0 => Ok(take(self.free, out_ptr, out_len)),
            _ => Err(take_error(self.free, out_ptr, out_len)),
        }))
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
        args: Args,
        _cursor: Option<String>,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>> {
        let input = encode_args(&args)?;
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
            return Ok(Err(take_error(self.free, out_ptr, out_len)));
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
            0 => Some(Ok(StreamItem {
                value: Value::encoded(take(self.task.free, out_ptr, out_len)),
                cursor: None,
            })),
            _ => Some(Err(take_error(self.task.free, out_ptr, out_len))),
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
            return Ok(Err(take_error(self.free, out_ptr, out_len)));
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
    fn step(&mut self, admit: Vec<(u64, Args)>) -> anyhow::Result<Finished> {
        let admit = admit
            .into_iter()
            .map(|(id, args)| Ok((id, ByteBuf::from(encode_args(&args)?.to_vec()))))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let input = rmp_serde::to_vec(&admit)?;
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
        if code != 0 {
            anyhow::bail!("{}", take_error(self.task.free, out_ptr, out_len));
        }
        let output = take(self.task.free, out_ptr, out_len);
        let finished: Vec<(u64, Result<ByteBuf, String>)> = rmp_serde::from_slice(&output)?;
        Ok(finished
            .into_iter()
            .map(|(id, output)| (id, super::output(output.map(ByteBuf::into_vec))))
            .collect())
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        // SAFETY: the session is open and closed exactly once.
        unsafe { (self.task.close)(self.handle) };
    }
}
