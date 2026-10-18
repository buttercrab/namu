//! Runs tasks compiled as native shared libraries exporting the NAMU C ABI (version 3). Every
//! buffer the library hands over is owned by it and given back through `namu_task_free`.
//!
//! - A task exports `namu_task_call`, and runs exactly once per call.
//! - A stream task exports `namu_stream_open`, `namu_stream_next` and `namu_stream_close`.

use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;

use libloading::Library;
use namu_engine::tasks::{StreamItem, TaskStream};

type AbiFn = unsafe extern "C" fn() -> u32;
type FreeFn = unsafe extern "C" fn(*mut u8, usize);
type CallFn = unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32;
type OpenFn =
    unsafe extern "C" fn(*const u8, usize, *mut *mut c_void, *mut *mut u8, *mut usize) -> i32;
type NextFn = unsafe extern "C" fn(*mut c_void, *mut *mut u8, *mut usize) -> i32;
type CloseFn = unsafe extern "C" fn(*mut c_void);

const ABI_VERSION: u32 = 3;

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

    pub fn call(&self, input: &str) -> anyhow::Result<Result<String, String>> {
        let (mut out_ptr, mut out_len) = (std::ptr::null_mut(), 0);
        // SAFETY: the ABI contract: the library writes an owned buffer, which `take` frees.
        let code = unsafe { (self.call)(input.as_ptr(), input.len(), &mut out_ptr, &mut out_len) };
        let output = take(self.free, out_ptr, out_len);
        Ok(if code == 0 { Ok(output) } else { Err(output) })
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

    pub fn open(
        self: &Arc<Self>,
        input: &str,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>> {
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
        let task = self.clone();
        Ok(Ok(Box::new(NativeItems { task, handle })))
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
