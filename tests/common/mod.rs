//! Common test utilities, tasks, and setup for graph integration tests.

use anyhow::Result;
use namu::task;
use namu_core::Value;
use namu_core::ir::Workflow;
use namu_engine::context::dynamic_context::DynamicContextManager;
use namu_engine::engine::simple_engine::SimpleEngine;
use namu_engine::engine::{Engine, PackFn, TaskImpl, UnpackFn};

// --- Test Tasks ---

#[task]
pub fn add(a: i32, b: i32) -> Result<i32> {
    Ok(a + b)
}

#[task]
pub fn is_positive(v: i32) -> Result<bool> {
    Ok(v > 0)
}

#[task]
pub fn double(v: i32) -> Result<i32> {
    Ok(v * 2)
}

#[task]
pub fn identity(v: i32) -> Result<i32> {
    Ok(v)
}

#[task]
pub fn is_negative(v: i32) -> Result<bool> {
    Ok(v < 0)
}

#[task]
pub fn less_than(a: i32, b: i32) -> Result<bool> {
    Ok(a < b)
}

#[task]
pub fn is_even(n: i32) -> Result<bool> {
    Ok(n % 2 == 0)
}

#[task]
pub fn divide_by_2(n: i32) -> Result<i32> {
    Ok(n / 2)
}

#[task]
pub fn multiply_by_3_and_add_1(n: i32) -> Result<i32> {
    Ok(n * 3 + 1)
}

#[task]
pub fn not_one(n: i32) -> Result<bool> {
    Ok(n != 1)
}

#[task(stream)]
pub fn range(start: i32, end: i32) -> Result<impl Iterator<Item = Result<i32>>> {
    Ok((start..end).map(|x| x * 10).map(Ok))
}

#[task(stream)]
pub fn split(n: i32, k: i32) -> Result<impl Iterator<Item = Result<i32>>> {
    Ok((0..k).map(move |x| n + x).map(Ok))
}

#[task]
#[allow(unreachable_code)]
pub fn panicker() -> Result<i32> {
    panic!("This should not be called!");
}

/// Convenience helper to execute a workflow IR with a freshly instantiated
/// in-process [`SimpleEngine`].
///
/// * `workflow` – The compiled workflow IR to run.
/// * `task_registrations` – An iterator over task registrations where each
///   tuple contains `(task_name, task_impl, pack_fn, unpack_fn)` mirroring
///   the parameters of [`Engine::add_task`].  Passing `None` for the pack / unpack
///   functions is fine for tasks that take / return a single value.
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
pub fn run_workflow<T>(workflow: Workflow, task_registrations: T) -> Vec<Value>
where
    T: IntoIterator<
        Item = (
            &'static str,
            TaskImpl<DynamicContextManager>,
            Option<PackFn>,
            Option<UnpackFn>,
        ),
    >,
{
    // 1. Spin up a new engine instance with the default dynamic context manager.
    let engine: SimpleEngine<DynamicContextManager> =
        SimpleEngine::new(DynamicContextManager::new());

    // 2. Register every provided task implementation.
    for (name, task_impl, pack, unpack) in task_registrations {
        engine.add_task(name, task_impl, pack, unpack);
    }

    // 3. Create a workflow run.
    let wf_id = engine.create_workflow(workflow);
    let run_id = engine.create_run(wf_id);

    // 4. Run the engine in a background thread so we can synchronously wait
    //    for the result on the main thread.
    let engine_clone = engine.clone();
    let handle = std::thread::spawn(move || {
        engine_clone.run(run_id);
    });

    // 5. Await completion and fetch the single output value.
    let result_rx = engine.get_result(run_id);

    let mut values = Vec::new();
    while let Ok(value) = result_rx.recv() {
        values.push(value);
    }

    handle.join().expect("engine thread panicked");

    values
}
