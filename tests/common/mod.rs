//! Common test utilities, tasks, and setup for graph integration tests.

#![allow(dead_code)]

use anyhow::Result;
use namu::{register_task, task};
use namu_core::Value;
use namu_core::ir::Workflow;
use namu_engine::context::dynamic_context::DynamicContextManager;
use namu_engine::engine::Engine;
use namu_engine::engine::simple_engine::SimpleEngine;

// --- Test Tasks ---

#[task(single)]
pub fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}

register_task! { method = add, name = "add", author = "test", version = "0.1" }

#[task(single)]
pub fn is_positive(v: i32) -> Result<bool> {
    Ok(v > 0)
}

#[task(single)]
pub fn double(v: i32) -> Result<i32> {
    Ok(v * 2)
}

#[task(single)]
pub fn identity(v: i32) -> Result<i32> {
    Ok(v)
}

#[task(single)]
pub fn is_negative(v: i32) -> Result<bool> {
    Ok(v < 0)
}

#[task(single)]
pub fn less_than(a: i32, b: i32) -> Result<bool> {
    Ok(a < b)
}

register_task! { method = less_than, name = "less_than", author = "test", version = "0.1" }

#[task(single)]
pub fn is_even(n: i32) -> Result<bool> {
    Ok(n % 2 == 0)
}

#[task(single)]
pub fn divide_by_2(n: i32) -> Result<i32> {
    Ok(n / 2)
}

#[task(single)]
pub fn multiply_by_3_and_add_1(n: i32) -> Result<i32> {
    Ok(n * 3 + 1)
}

#[task(single)]
pub fn not_one(n: i32) -> Result<bool> {
    Ok(n != 1)
}

#[task(stream)]
pub fn range(start: i32, end: i32) -> Result<impl Iterator<Item = Result<i32>>> {
    Ok((start..end).map(|x| x * 10).map(Ok))
}

register_task! { method = range, name = "range", author = "test", version = "0.1" }

#[task(stream)]
pub fn split(n: i32, k: i32) -> Result<impl Iterator<Item = Result<i32>>> {
    Ok((0..k).map(move |x| n + x).map(Ok))
}

register_task! { method = split, name = "split", author = "test", version = "0.1" }

#[task(single)]
#[allow(unreachable_code)]
pub fn panicker() -> Result<i32> {
    panic!("This should not be called!");
}

/// Convenience helper to execute a workflow IR with a freshly instantiated
/// in-process [`SimpleEngine`].
///
/// * `workflow` – The compiled workflow IR to run.
/// * `task_registrations` – An iterator over task registrations where each tuple contains
///   `(task_name, task_impl, pack_fn, unpack_fn)` mirroring the parameters of [`Engine::add_task`].
///   Passing `None` for the pack / unpack functions is fine for tasks that take / return a single
///   value.
///
/// The function blocks until the workflow finishes and returns the produced
/// [`Value`].  The caller is expected to down-cast the value to the expected
/// concrete type:
///
/// ```ignore
/// let out = run_workflow(wf_ir, [
///     ("add", Box::new(__add), Some(pack_add as PackFn), None),
/// ]);
/// let result = *out.downcast_ref::<i32>().unwrap();
/// assert_eq!(result, 3);
/// ```
pub fn run_workflow(workflow: Workflow) -> Vec<Value> {
    // 1. Spin up engine that auto-registers tasks collected via inventory.
    let engine: SimpleEngine<DynamicContextManager> =
        SimpleEngine::with_registered(DynamicContextManager::new());

    // 2. Create a workflow run.
    let wf_id = engine.create_workflow(workflow);
    let run_id = engine.create_run(wf_id);

    // 3. Run the engine in a background thread so we can synchronously wait for the result on the
    //    main thread.
    let engine_clone = engine.clone();
    let handle = std::thread::spawn(move || {
        engine_clone.run(run_id);
    });

    // 4. Await completion and fetch the single output value.
    let result_rx = engine.get_result(run_id);

    let values = result_rx.collect::<Vec<_>>();

    handle.join().expect("engine thread panicked");

    values
}
