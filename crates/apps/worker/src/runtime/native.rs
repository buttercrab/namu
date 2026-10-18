//! Runs tasks compiled as native shared libraries exporting the NAMU C ABI (version 2):
//! `namu_task_call` returns a buffer the library owns, which is handed back via
//! `namu_task_free`. The task runs exactly once per call.

use std::path::Path;

use libloading::Library;

type CallFn = unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32;
type FreeFn = unsafe extern "C" fn(*mut u8, usize);
type AbiFn = unsafe extern "C" fn() -> u32;

pub struct NativeTask {
    call: CallFn,
    free: FreeFn,
    // Kept loaded for the worker's lifetime: unloading native code is not reliably safe.
    _lib: Library,
}

impl NativeTask {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        // SAFETY: task code is trusted; the symbols are checked against the expected ABI version.
        unsafe {
            let lib = Library::new(path)?;
            let abi: AbiFn = *lib.get::<AbiFn>(b"namu_abi_version")?;
            let version = abi();
            if version != namu_guest_abi_version() {
                anyhow::bail!("unsupported native task ABI version {version}");
            }
            let call: CallFn = *lib.get::<CallFn>(b"namu_task_call")?;
            let free: FreeFn = *lib.get::<FreeFn>(b"namu_task_free")?;
            Ok(Self {
                call,
                free,
                _lib: lib,
            })
        }
    }

    pub fn call(&self, input: &str) -> anyhow::Result<Result<String, String>> {
        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        // SAFETY: the ABI contract: the library writes an owned buffer that we free once.
        let code = unsafe { (self.call)(input.as_ptr(), input.len(), &mut out_ptr, &mut out_len) };
        let output = if out_ptr.is_null() {
            String::new()
        } else {
            let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
            let text = String::from_utf8_lossy(bytes).into_owned();
            unsafe { (self.free)(out_ptr, out_len) };
            text
        };
        Ok(if code == 0 { Ok(output) } else { Err(output) })
    }
}

fn namu_guest_abi_version() -> u32 {
    2
}
