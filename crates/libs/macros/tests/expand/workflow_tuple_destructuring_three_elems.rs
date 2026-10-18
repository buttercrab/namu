use namu_macros::{task, workflow};

#[task]
fn triple(a: i32) -> anyhow::Result<(i32, bool, String)>;

#[workflow]
fn destructure_workflow() {
    let (x, y, z) = triple(42);
    // suppress unused vars
    let _ = (x, y, z);
}
