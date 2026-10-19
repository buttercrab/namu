//! Runs tasks compiled as WASM components (`namu:task` worlds). One compiled component is shared
//! by all calls; instances are pooled. A trap or panic only fails that call and that instance.

use std::path::Path;
use std::sync::{Arc, Mutex};

use namu_core::value::{Value, encode_args};
use namu_engine::tasks::{
    Args, BatchSession, BatchTask, CallKey, CallTask, Finished, StreamItem, StreamTask, TaskStream,
};
use wasmtime::component::{Component, InstancePre, Linker, ResourceAny, ResourceTable, TypedFunc};
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

type CallFunc = TypedFunc<(String, Vec<u8>), (Result<Vec<u8>, String>,)>;

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
            .map_err(|err| anyhow::anyhow!("compile wasm component {}: {err:?}", path.display()))?;
        let mut linker = Linker::<Host>::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
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
            .get_typed_func(&mut store, "call")
            .map_err(|err| anyhow::anyhow!("component does not export `call`: {err:?}"))?;
        Ok(Instance { store, func })
    }
}

impl CallTask for WasmTask {
    fn call(&self, args: Args, key: CallKey) -> anyhow::Result<Result<Value, String>> {
        let input = encode_args(&args)?;
        let pooled = self.idle.lock().unwrap().pop();
        let mut instance = match pooled {
            Some(instance) => instance,
            None => self.instantiate()?,
        };
        // On a trap the instance is dropped instead of returned to the pool.
        let (result,) = instance
            .func
            .call(&mut instance.store, (key.to_string(), input.to_vec()))?;
        self.idle.lock().unwrap().push(instance);
        Ok(super::output(result))
    }
}

mod source {
    wasmtime::component::bindgen!({
        path: "../../libs/guest/wit",
        world: "stream-task",
    });
}

/// A stream task compiled as a WASM component (`stream-task` world). Each open stream gets its
/// own instance, which lives as long as the stream.
pub struct WasmStreamTask {
    engine: Engine,
    pre: source::StreamTaskPre<Host>,
}

impl WasmStreamTask {
    pub fn load(engine: &Engine, path: &Path) -> anyhow::Result<Self> {
        let component = Component::from_file(engine, path)
            .map_err(|err| anyhow::anyhow!("compile wasm component {}: {err:?}", path.display()))?;
        let mut linker = Linker::<Host>::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        let pre = source::StreamTaskPre::new(linker.instantiate_pre(&component)?)?;
        Ok(Self {
            engine: engine.clone(),
            pre,
        })
    }
}

impl StreamTask for WasmStreamTask {
    fn open(
        self: Arc<Self>,
        args: Args,
        _cursor: Option<String>,
    ) -> anyhow::Result<Result<Box<dyn TaskStream>, String>> {
        let input = encode_args(&args)?;
        let host = Host {
            wasi: WasiCtx::builder().inherit_stderr().build(),
            table: ResourceTable::new(),
        };
        let mut store = Store::new(&self.engine, host);
        let instance = self.pre.instantiate(&mut store)?;
        let items = match instance.namu_task_source().call_open(&mut store, &input)? {
            Ok(items) => items,
            Err(message) => return Ok(Err(message)),
        };
        Ok(Ok(Box::new(WasmItems {
            store,
            instance,
            items: Some(items),
        })))
    }
}

struct WasmItems {
    store: Store<Host>,
    instance: source::StreamTask,
    /// Taken when the stream is dropped, to run the guest's destructor.
    items: Option<ResourceAny>,
}

impl TaskStream for WasmItems {
    fn next(&mut self) -> anyhow::Result<Option<Result<StreamItem, String>>> {
        let items = self.items.expect("the stream is open");
        let item = self
            .instance
            .namu_task_source()
            .items()
            .call_next(&mut self.store, items)?;
        Ok(item.map(|item| {
            Ok(StreamItem {
                value: Value::encoded(item?),
                cursor: None,
            })
        }))
    }
}

impl Drop for WasmItems {
    fn drop(&mut self) {
        if let Some(items) = self.items.take() {
            let _ = items.resource_drop(&mut self.store);
        }
    }
}

mod batch {
    wasmtime::component::bindgen!({
        path: "../../libs/guest/wit",
        world: "batch-task",
    });
}

/// A batch task compiled as a WASM component (`batch-task` world). Each session gets its own
/// instance, which lives as long as the session.
pub struct WasmBatchTask {
    engine: Engine,
    pre: batch::BatchTaskPre<Host>,
}

impl WasmBatchTask {
    pub fn load(engine: &Engine, path: &Path) -> anyhow::Result<Self> {
        let component = Component::from_file(engine, path)
            .map_err(|err| anyhow::anyhow!("compile wasm component {}: {err:?}", path.display()))?;
        let mut linker = Linker::<Host>::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        let pre = batch::BatchTaskPre::new(linker.instantiate_pre(&component)?)?;
        Ok(Self {
            engine: engine.clone(),
            pre,
        })
    }
}

impl BatchTask for WasmBatchTask {
    fn open(self: Arc<Self>) -> anyhow::Result<Result<Box<dyn BatchSession>, String>> {
        let host = Host {
            wasi: WasiCtx::builder().inherit_stderr().build(),
            table: ResourceTable::new(),
        };
        let mut store = Store::new(&self.engine, host);
        let instance = self.pre.instantiate(&mut store)?;
        let session = match instance.namu_task_batch().call_open(&mut store)? {
            Ok(session) => session,
            Err(message) => return Ok(Err(message)),
        };
        Ok(Ok(Box::new(WasmSession {
            store,
            instance,
            session: Some(session),
        })))
    }
}

struct WasmSession {
    store: Store<Host>,
    instance: batch::BatchTask,
    /// Taken when the session is dropped, to run the guest's destructor.
    session: Option<ResourceAny>,
}

impl BatchSession for WasmSession {
    fn step(&mut self, admit: Vec<(u64, Args)>) -> anyhow::Result<Finished> {
        use batch::exports::namu::task::batch::Admitted;

        let session = self.session.expect("the session is open");
        let admit = admit
            .into_iter()
            .map(|(id, args)| {
                let input = encode_args(&args)?.to_vec();
                Ok(Admitted { id, input })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let finished = self
            .instance
            .namu_task_batch()
            .session()
            .call_step(&mut self.store, session, &admit)?
            .map_err(|err| anyhow::anyhow!(err))?;
        Ok(finished
            .into_iter()
            .map(|item| (item.id, super::output(item.output)))
            .collect())
    }
}

impl Drop for WasmSession {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = session.resource_drop(&mut self.store);
        }
    }
}
