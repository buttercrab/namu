use std::any::TypeId;
use std::fmt::{Display, Formatter};
use std::mem;

use anyhow::Result;
use async_trait::async_trait;
use kanal::{ReceiveError, Receiver, SendError, Sender};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{ContextId, Value};

#[derive(Debug, Copy, Clone)]
pub struct TaskEnd;

impl Display for TaskEnd {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "TaskEnd")
    }
}

impl std::error::Error for TaskEnd {}

#[async_trait]
pub trait TaskContext: Send {
    fn recv<T: Send + DeserializeOwned + 'static>(&self) -> Result<(ContextId, T), ReceiveError>;

    async fn recv_async<T: Send + DeserializeOwned + 'static>(
        &self,
    ) -> Result<(ContextId, T), ReceiveError>;

    fn try_recv<T: Send + DeserializeOwned + 'static>(
        &self,
    ) -> Result<Option<(ContextId, T)>, ReceiveError>;

    fn send<T: Send + Serialize + Clone + Send + Sync + 'static>(
        &self,
        item_id: ContextId,
        data: Result<T>,
    ) -> Result<(), SendError>;

    async fn send_async<T: Send + Serialize + Clone + Send + Sync + 'static>(
        &self,
        item_id: ContextId,
        data: Result<T>,
    ) -> Result<(), SendError>;

    fn send_end(&self, item_id: ContextId) -> Result<(), SendError>;

    async fn send_end_async(&self, item_id: ContextId) -> Result<(), SendError>;
}

#[derive(Clone)]
pub struct StaticTaskContext<In, Out> {
    input_ch: Receiver<(ContextId, In)>,
    output_ch: Sender<(ContextId, Result<Out>)>,
}

impl<In, Out> StaticTaskContext<In, Out> {
    pub fn new(
        input_ch: Receiver<(ContextId, In)>,
        output_ch: Sender<(ContextId, Result<Out>)>,
    ) -> Self {
        Self {
            input_ch,
            output_ch,
        }
    }
}

#[async_trait]
impl<In, Out> TaskContext for StaticTaskContext<In, Out>
where
    In: Send + 'static,
    Out: Send + 'static,
{
    fn recv<T: 'static>(&self) -> Result<(ContextId, T), ReceiveError> {
        assert_eq!(
            TypeId::of::<T>(),
            TypeId::of::<In>(),
            "StaticTaskContext received a request for the wrong type. Expected {}, got {}.",
            std::any::type_name::<In>(),
            std::any::type_name::<T>()
        );

        let (id, data) = self.input_ch.recv()?;

        // SAFETY: We know that the type of the data is the same as the type of the input.
        Ok((id, unsafe { transmute_unchecked(data) }))
    }

    async fn recv_async<T: 'static>(&self) -> Result<(ContextId, T), ReceiveError> {
        assert_eq!(
            TypeId::of::<T>(),
            TypeId::of::<In>(),
            "StaticTaskContext received a request for the wrong type. Expected {}, got {}.",
            std::any::type_name::<In>(),
            std::any::type_name::<T>()
        );

        let (id, data) = self.input_ch.as_async().recv().await?;
        Ok((id, unsafe { transmute_unchecked(data) }))
    }

    fn try_recv<T: 'static>(&self) -> Result<Option<(ContextId, T)>, ReceiveError> {
        assert_eq!(
            TypeId::of::<T>(),
            TypeId::of::<In>(),
            "StaticTaskContext received a request for the wrong type. Expected {}, got {}.",
            std::any::type_name::<In>(),
            std::any::type_name::<T>()
        );

        // SAFETY: We know that the type of the data is the same as the type of the input.
        let result = self
            .input_ch
            .try_recv()?
            .map(|(id, data)| (id, unsafe { transmute_unchecked(data) }));
        Ok(result)
    }

    fn send<T: 'static>(&self, item_id: ContextId, data: Result<T>) -> Result<(), SendError> {
        assert_eq!(
            TypeId::of::<T>(),
            TypeId::of::<Out>(),
            "StaticTaskContext received a send request for the wrong type. Expected {}, got {}.",
            std::any::type_name::<Out>(),
            std::any::type_name::<T>()
        );

        // SAFETY: We know that the type of the data is the same as the type of the output.
        self.output_ch.send((
            item_id,
            data.map(|data| unsafe { transmute_unchecked(data) }),
        ))
    }

    async fn send_async<T: Send + 'static>(
        &self,
        item_id: ContextId,
        data: Result<T>,
    ) -> Result<(), SendError> {
        assert_eq!(
            TypeId::of::<T>(),
            TypeId::of::<Out>(),
            "StaticTaskContext received a send request for the wrong type. Expected {}, got {}.",
            std::any::type_name::<Out>(),
            std::any::type_name::<T>()
        );

        self.output_ch
            .as_async()
            .send((
                item_id,
                data.map(|data| unsafe { transmute_unchecked(data) }),
            ))
            .await
    }

    fn send_end(&self, item_id: ContextId) -> Result<(), SendError> {
        // TODO: better way to signal end of task?
        self.output_ch
            .send((item_id, Err(anyhow::Error::from(TaskEnd))))
    }

    async fn send_end_async(&self, item_id: ContextId) -> Result<(), SendError> {
        self.output_ch
            .as_async()
            .send((item_id, Err(anyhow::Error::from(TaskEnd))))
            .await
    }
}

#[derive(Clone)]
pub struct DynamicTaskContext {
    input_ch: Receiver<(ContextId, Value)>,
    output_ch: Sender<(ContextId, Result<Value>)>,
}

impl DynamicTaskContext {
    pub fn new(
        input_ch: Receiver<(ContextId, Value)>,
        output_ch: Sender<(ContextId, Result<Value>)>,
    ) -> Self {
        Self {
            input_ch,
            output_ch,
        }
    }
}

#[async_trait]
impl TaskContext for DynamicTaskContext {
    fn recv<T: 'static>(&self) -> Result<(ContextId, T), ReceiveError> {
        let (id, data) = self.input_ch.recv()?;
        Ok((id, unsafe { data.take::<T>() }))
    }

    async fn recv_async<T: 'static>(&self) -> Result<(ContextId, T), ReceiveError> {
        let (id, data) = self.input_ch.as_async().recv().await?;
        Ok((id, unsafe { data.take::<T>() }))
    }

    fn try_recv<T: 'static>(&self) -> Result<Option<(ContextId, T)>, ReceiveError> {
        let result = self
            .input_ch
            .try_recv()?
            .map(|(id, data)| (id, unsafe { data.take::<T>() }));
        Ok(result)
    }

    fn send<T: Serialize + Clone + Send + Sync + 'static>(
        &self,
        item_id: ContextId,
        data: Result<T>,
    ) -> Result<(), SendError> {
        self.output_ch.send((item_id, data.map(Value::new)))
    }

    async fn send_async<T: Serialize + Clone + Send + Sync + 'static>(
        &self,
        item_id: ContextId,
        data: Result<T>,
    ) -> Result<(), SendError> {
        self.output_ch
            .as_async()
            .send((item_id, data.map(Value::new)))
            .await
    }

    fn send_end(&self, item_id: ContextId) -> Result<(), SendError> {
        // TODO: better way to signal end of task?
        self.output_ch
            .send((item_id, Err(anyhow::Error::from(TaskEnd))))
    }

    async fn send_end_async(&self, item_id: ContextId) -> Result<(), SendError> {
        self.output_ch
            .as_async()
            .send((item_id, Err(anyhow::Error::from(TaskEnd))))
            .await
    }
}

// https://github.com/funnsam/stable-intrinsics/blob/586a139ccb758488f109daf5165ecef574723b3e/src/lib.rs#L181
unsafe fn transmute_unchecked<Src, Dst>(src: Src) -> Dst {
    let dst = unsafe { mem::transmute::<*const Src, *const Dst>(&src as *const Src) };
    mem::forget(src);
    unsafe { dst.read() }
}
