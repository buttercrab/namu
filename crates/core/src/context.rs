use anyhow::Result;
use async_trait::async_trait;
use kanal::{ReceiveError, Receiver, SendError, Sender};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    any::{Any, TypeId},
    fmt::{Display, Formatter},
    mem,
};

#[derive(Debug, Copy, Clone)]
struct TaskEnd;

impl Display for TaskEnd {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "TaskEnd")
    }
}

impl std::error::Error for TaskEnd {}

#[async_trait]
pub trait TaskContext<Id>: Send {
    fn recv<T: Send + DeserializeOwned + 'static>(&self) -> Result<(Id, T), ReceiveError>;

    async fn recv_async<T: Send + DeserializeOwned + 'static>(
        &self,
    ) -> Result<(Id, T), ReceiveError>;

    fn try_recv<T: Send + DeserializeOwned + 'static>(
        &self,
    ) -> Result<Option<(Id, T)>, ReceiveError>;

    fn send<T: Send + Serialize + 'static>(
        &self,
        item_id: Id,
        data: Result<T>,
    ) -> Result<(), SendError>;

    async fn send_async<T: Send + Serialize + 'static>(
        &self,
        item_id: Id,
        data: Result<T>,
    ) -> Result<(), SendError>;

    fn send_end(&self, item_id: Id) -> Result<(), SendError>;

    async fn send_end_async(&self, item_id: Id) -> Result<(), SendError>;
}

#[derive(Clone)]
pub struct StaticTaskContext<Id, In, Out> {
    input_ch: Receiver<(Id, In)>,
    output_ch: Sender<(Id, Result<Out>)>,
}

impl<Id, In, Out> StaticTaskContext<Id, In, Out> {
    pub fn new(input_ch: Receiver<(Id, In)>, output_ch: Sender<(Id, Result<Out>)>) -> Self {
        Self {
            input_ch,
            output_ch,
        }
    }
}

#[async_trait]
impl<'de, Id, In, Out> TaskContext<Id> for StaticTaskContext<Id, In, Out>
where
    Id: Send,
    In: Send + 'static,
    Out: Send + 'static,
{
    fn recv<T: 'static>(&self) -> Result<(Id, T), ReceiveError> {
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

    async fn recv_async<T: 'static>(&self) -> Result<(Id, T), ReceiveError> {
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

    fn try_recv<T: 'static>(&self) -> Result<Option<(Id, T)>, ReceiveError> {
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

    fn send<T: 'static>(&self, item_id: Id, data: Result<T>) -> Result<(), SendError> {
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
        item_id: Id,
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

    fn send_end(&self, item_id: Id) -> Result<(), SendError> {
        // TODO: better way to signal end of task?
        self.output_ch
            .send((item_id, Err(anyhow::Error::from(TaskEnd))))
    }

    async fn send_end_async(&self, item_id: Id) -> Result<(), SendError> {
        self.output_ch
            .as_async()
            .send((item_id, Err(anyhow::Error::from(TaskEnd))))
            .await
    }
}

#[derive(Clone)]
pub struct DynamicTaskContext<Id> {
    input_ch: Receiver<(Id, Box<dyn Any + Send>)>,
    output_ch: Sender<(Id, Result<Box<dyn Any + Send>>)>,
}

impl<Id> DynamicTaskContext<Id> {
    pub fn new(
        input_ch: Receiver<(Id, Box<dyn Any + Send>)>,
        output_ch: Sender<(Id, Result<Box<dyn Any + Send>>)>,
    ) -> Self {
        Self {
            input_ch,
            output_ch,
        }
    }
}

#[async_trait]
impl<Id> TaskContext<Id> for DynamicTaskContext<Id>
where
    Id: Send,
{
    fn recv<T: 'static>(&self) -> Result<(Id, T), ReceiveError> {
        let (id, data) = self.input_ch.recv()?;
        Ok((id, *data.downcast::<T>().unwrap()))
    }

    async fn recv_async<T: 'static>(&self) -> Result<(Id, T), ReceiveError> {
        let (id, data) = self.input_ch.as_async().recv().await?;
        Ok((id, *data.downcast::<T>().unwrap()))
    }

    fn try_recv<T: 'static>(&self) -> Result<Option<(Id, T)>, ReceiveError> {
        let result = self
            .input_ch
            .try_recv()?
            .map(|(id, data)| (id, *data.downcast::<T>().unwrap()));
        Ok(result)
    }

    fn send<T: Send + 'static>(&self, item_id: Id, data: Result<T>) -> Result<(), SendError> {
        self.output_ch.send((
            item_id,
            data.map(|data| Box::new(data) as Box<dyn Any + Send>),
        ))
    }

    async fn send_async<T: Send + 'static>(
        &self,
        item_id: Id,
        data: Result<T>,
    ) -> Result<(), SendError> {
        self.output_ch
            .as_async()
            .send((
                item_id,
                data.map(|data| Box::new(data) as Box<dyn Any + Send>),
            ))
            .await
    }

    fn send_end(&self, item_id: Id) -> Result<(), SendError> {
        // TODO: better way to signal end of task?
        self.output_ch
            .send((item_id, Err(anyhow::Error::from(TaskEnd))))
    }

    async fn send_end_async(&self, item_id: Id) -> Result<(), SendError> {
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
