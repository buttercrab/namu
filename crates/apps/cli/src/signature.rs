//! A task's signature, read from what was built, for its manifest: a Rust task says it itself (its
//! native build exports `namu_signature`), and a Python task's is read from its annotations by
//! the SDK's reader, which parses the module without running it.

use std::path::Path;
use std::process::Command;

use anyhow::Context;
use libloading::{Library, Symbol};
use namu_core::signature::Signature;

/// The SDK's reader of Python signatures (`namu.signature`), run as a script.
const PYTHON_READER: &str = include_str!("../../../../sdk/python/namu/signature.py");

type SignatureFn = unsafe extern "C" fn(*mut *mut u8, *mut usize) -> i32;
type FreeFn = unsafe extern "C" fn(*mut u8, usize);

/// The signature a task's native library exports.
pub fn native(library: &Path) -> anyhow::Result<Signature> {
    let what = || format!("the signature of {}", library.display());
    // SAFETY: the library is the task's own crate, just built by `namu build` with `namu-guest`,
    // whose exports have these types; loading it runs no code of the task's.
    let signature = unsafe {
        let loaded = Library::new(library).with_context(what)?;
        let read: Symbol<SignatureFn> = loaded.get(b"namu_signature").with_context(|| {
            format!(
                "{} exports no signature: build it with this version of namu-guest",
                library.display()
            )
        })?;
        let free: Symbol<FreeFn> = loaded.get(b"namu_task_free").with_context(what)?;
        let (mut ptr, mut len) = (std::ptr::null_mut(), 0);
        let code = read(&mut ptr, &mut len);
        let bytes = std::slice::from_raw_parts(ptr, len).to_vec();
        free(ptr, len);
        anyhow::ensure!(code == 0, "{}: {}", what(), String::from_utf8_lossy(&bytes));
        bytes
    };
    rmp_serde::from_slice(&signature).with_context(what)
}

/// The signature of `task` in the Python module at `module`, or `None` if there is no Python to
/// read it with (`NAMU_PYTHON`, default `python3`).
pub fn python(module: &Path, task: &str) -> anyhow::Result<Option<Signature>> {
    let python = std::env::var("NAMU_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let output = match Command::new(&python)
        .args(["-I", "-c", PYTHON_READER])
        .arg(module)
        .arg(task)
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("warning: {python} is not found, to read Python signatures with");
            return Ok(None);
        }
        Err(err) => return Err(err).context(format!("running {python}")),
    };
    anyhow::ensure!(
        output.status.success(),
        "reading the signature of {task} in {}: {}",
        module.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    serde_json::from_slice(&output.stdout)
        .map(Some)
        .with_context(|| format!("the signature of {task} in {}", module.display()))
}
