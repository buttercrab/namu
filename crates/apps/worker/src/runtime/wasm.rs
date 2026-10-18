//! Runs tasks compiled as WASM components (`namu:task` world). One compiled component is shared
//! by all calls; instances are pooled. A trap or panic only fails that call and that instance.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Context;
use wasmtime::component::{Component, InstancePre, Linker, ResourceTable, TypedFunc};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

type CallFunc = TypedFunc<(String,), (Result<String, String>,)>;

/// A live instance. Instances are reused across calls, so a task keeps its state (e.g. a loaded
/// model) between calls; an instance that traps is discarded.
struct Instance {
    store: Store<Host>,
    func: CallFunc,
}

pub struct WasmTask {
    engine: Engine,
    pre: InstancePre<Host>,
    idle: Mutex<Vec<Instance>>,
}

impl WasmTask {
    pub fn load(engine: &Engine, path: &Path) -> anyhow::Result<Self> {
        // Compiled once per worker; the artifact itself is platform-independent.
        let component = Component::from_file(engine, path)
            .with_context(|| format!("compile wasm component {}", path.display()))?;
        let mut linker = Linker::<Host>::new(engine);
        // Recent Rust std targets import the (still unstable) `exit-with-code`.
        let mut options = wasmtime_wasi::p2::bindings::sync::LinkOptions::default();
        options.cli_exit_with_code(true);
        wasmtime_wasi::p2::add_to_linker_with_options_sync(&mut linker, &options)?;
        let pre = linker.instantiate_pre(&component)?;
        Ok(Self {
            engine: engine.clone(),
            pre,
            idle: Mutex::new(Vec::new()),
        })
    }

    fn instantiate(&self) -> anyhow::Result<Instance> {
        let host = Host {
            wasi: WasiCtx::builder().inherit_stderr().build(),
            table: ResourceTable::new(),
        };
        let mut store = Store::new(&self.engine, host);
        let instance = self.pre.instantiate(&mut store)?;
        let func = instance
            .get_typed_func::<(String,), (Result<String, String>,)>(&mut store, "call")
            .context("component does not export `call`")?;
        Ok(Instance { store, func })
    }

    /// Calls the task. The outer error is an infrastructure failure (trap, bad artifact); the
    /// inner one is the task's own error.
    pub fn call(&self, input: &str) -> anyhow::Result<Result<String, String>> {
        let pooled = self.idle.lock().unwrap().pop();
        let mut instance = match pooled {
            Some(instance) => instance,
            None => self.instantiate()?,
        };
        // On a trap the instance is dropped instead of returned to the pool.
        let (result,) = instance
            .func
            .call(&mut instance.store, (input.to_string(),))?;
        instance.func.post_return(&mut instance.store)?;
        self.idle.lock().unwrap().push(instance);
        Ok(result)
    }
}
