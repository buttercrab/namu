use std::ffi::c_void;

#[unsafe(no_mangle)]
pub extern "C" fn namu_task_create() -> *mut c_void {
    Box::into_raw(Box::new(())) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn namu_task_destroy(handle: *mut c_void) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle as *mut ())) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn namu_task_call(
    _handle: *mut c_void,
    input_ptr: *const u8,
    input_len: usize,
    output_ptr: *mut u8,
    output_len: *mut usize,
) -> i32 {
    let input = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let parsed: serde_json::Value = match serde_json::from_slice(input) {
        Ok(value) => value,
        Err(err) => return write_error(err.to_string(), output_ptr, output_len),
    };

    let array = match parsed.as_array() {
        Some(array) if array.len() == 2 => array,
        _ => {
            return write_error("expected [a, b]".to_string(), output_ptr, output_len);
        }
    };

    let a = array[0].as_i64().unwrap_or(0);
    let b = array[1].as_i64().unwrap_or(0);
    let out = serde_json::Value::from(a + b).to_string();

    write_output(out.as_bytes(), output_ptr, output_len)
}

fn write_output(bytes: &[u8], output_ptr: *mut u8, output_len: *mut usize) -> i32 {
    unsafe {
        let capacity = *output_len;
        if capacity < bytes.len() {
            *output_len = bytes.len();
            return 1;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), output_ptr, bytes.len());
        *output_len = bytes.len();
    }
    0
}

fn write_error(message: String, output_ptr: *mut u8, output_len: *mut usize) -> i32 {
    let payload = serde_json::json!({
        "error": { "message": message, "kind": "TaskError" }
    })
    .to_string();
    write_output(payload.as_bytes(), output_ptr, output_len)
}
