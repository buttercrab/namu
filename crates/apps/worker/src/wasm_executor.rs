use std::path::Path;

use anyhow::Context;
use serde_json::Value as JsonValue;
use wasmtime::{Engine, Linker, Memory, Module, Store, TypedFunc};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::{self, WasiP1Ctx};

pub fn call_task(path: &Path, input_json: &JsonValue) -> anyhow::Result<Result<JsonValue, String>> {
    let engine = Engine::default();
    let module = Module::from_file(&engine, path)
        .with_context(|| format!("failed to load wasm module at {}", path.to_string_lossy()))?;

    let mut linker: Linker<WasiState> = Linker::new(&engine);
    p1::add_to_linker_sync(&mut linker, |state| &mut state.wasi)?;

    let wasi = WasiCtxBuilder::new().build_p1();
    let mut store = Store::new(&engine, WasiState { wasi });

    let instance = linker
        .instantiate(&mut store, &module)
        .context("instantiate wasm module")?;

    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| anyhow::anyhow!("missing wasm memory export"))?;

    let create = instance
        .get_typed_func::<(), i32>(&mut store, "namu_task_create")
        .context("missing namu_task_create")?;
    let destroy = instance
        .get_typed_func::<i32, ()>(&mut store, "namu_task_destroy")
        .context("missing namu_task_destroy")?;
    let call = instance
        .get_typed_func::<(i32, i32, i32, i32, i32), i32>(&mut store, "namu_task_call")
        .context("missing namu_task_call")?;

    let handle = create.call(&mut store, ())?;
    let input_bytes = serde_json::to_vec(input_json)?;
    let mut output_capacity = 4096usize;

    let (mut code, out_len, mut output) = call_once(
        &mut store,
        &memory,
        &call,
        handle,
        &input_bytes,
        output_capacity,
    )?;

    if code != 0 && out_len > output_capacity {
        output_capacity = out_len;
        let retry = call_once(
            &mut store,
            &memory,
            &call,
            handle,
            &input_bytes,
            output_capacity,
        )?;
        code = retry.0;
        output = retry.2;
    }

    destroy.call(&mut store, handle)?;

    let output_str = std::str::from_utf8(&output).unwrap_or("");
    if code == 0 {
        let json = serde_json::from_str(output_str)?;
        Ok(Ok(json))
    } else {
        let err = if output_str.is_empty() {
            "task failed".to_string()
        } else {
            output_str.to_string()
        };
        Ok(Err(err))
    }
}

struct WasiState {
    wasi: WasiP1Ctx,
}

fn call_once(
    store: &mut Store<WasiState>,
    memory: &Memory,
    call: &TypedFunc<(i32, i32, i32, i32, i32), i32>,
    handle: i32,
    input: &[u8],
    output_capacity: usize,
) -> anyhow::Result<(i32, usize, Vec<u8>)> {
    let input_len = input.len();
    let input_offset = 0usize;
    let output_offset = align_up(input_offset + input_len, 8);
    let output_len_offset = align_up(output_offset + output_capacity, 8);
    let required = output_len_offset + std::mem::size_of::<u32>();

    ensure_memory(memory, store, required)?;

    {
        let data = memory.data_mut(&mut *store);
        data[input_offset..input_offset + input_len].copy_from_slice(input);
        write_u32(data, output_len_offset, output_capacity as u32);
    }

    let code = call.call(
        &mut *store,
        (
            handle,
            input_offset as i32,
            input_len as i32,
            output_offset as i32,
            output_len_offset as i32,
        ),
    )?;

    let (out_len, output) = {
        let data = memory.data(&mut *store);
        let out_len = read_u32(data, output_len_offset) as usize;
        let end = output_offset + out_len.min(output_capacity);
        (out_len, data[output_offset..end].to_vec())
    };

    Ok((code, out_len, output))
}

fn ensure_memory(
    memory: &Memory,
    store: &mut Store<WasiState>,
    required: usize,
) -> anyhow::Result<()> {
    let current = memory.data_size(&mut *store);
    if current >= required {
        return Ok(());
    }
    let delta = required - current;
    let pages = delta.div_ceil(WASM_PAGE_SIZE);
    memory
        .grow(&mut *store, pages as u64)
        .context("failed to grow wasm memory")?;
    Ok(())
}

fn align_up(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}

fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    buf[offset..offset + 4].copy_from_slice(&bytes);
}

fn read_u32(buf: &[u8], offset: usize) -> u32 {
    let mut out = [0u8; 4];
    out.copy_from_slice(&buf[offset..offset + 4]);
    u32::from_le_bytes(out)
}

const WASM_PAGE_SIZE: usize = 65536;
