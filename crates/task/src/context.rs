use crate::ItemId;
use anyhow::Result;
use kanal::{ReceiveError, Receiver, SendError, Sender};
use std::{
    any::{Any, TypeId},
    mem,
};

pub trait TaskContext: Send + Sync {
    fn recv<T: Send + Sync + 'static + Clone>(&self) -> Result<(ItemId, T), ReceiveError>;

    fn try_recv<T: Send + Sync + 'static + Clone>(
        &self,
    ) -> Result<Option<(ItemId, T)>, ReceiveError>;

    fn send<T: Send + Sync + 'static>(
        &self,
        item_id: ItemId,
        data: Result<T>,
    ) -> Result<(), SendError>;
}

pub struct StaticTaskContext<In, Out> {
    input_ch: Receiver<(ItemId, In)>,
    output_ch: Sender<(ItemId, Result<Out>)>,
}

impl<In, Out> StaticTaskContext<In, Out> {
    pub fn new(input_ch: Receiver<(ItemId, In)>, output_ch: Sender<(ItemId, Result<Out>)>) -> Self {
        Self {
            input_ch,
            output_ch,
        }
    }
}

impl<In, Out> Clone for StaticTaskContext<In, Out> {
    fn clone(&self) -> Self {
        Self {
            input_ch: self.input_ch.clone(),
            output_ch: self.output_ch.clone(),
        }
    }
}

impl<In, Out> TaskContext for StaticTaskContext<In, Out>
where
    In: Send + Sync + 'static + Clone,
    Out: Send + Sync + 'static,
{
    fn recv<T: Send + Sync + 'static + Clone>(&self) -> Result<(ItemId, T), ReceiveError> {
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

    fn try_recv<T: Send + Sync + 'static + Clone>(
        &self,
    ) -> Result<Option<(ItemId, T)>, ReceiveError> {
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

    fn send<T: Send + Sync + 'static>(
        &self,
        item_id: ItemId,
        data: Result<T>,
    ) -> Result<(), SendError> {
        assert_eq!(
            TypeId::of::<T>(),
            TypeId::of::<Out>(),
            "StaticTaskContext received a send request for the wrong type. Expected {}, got {}.",
            std::any::type_name::<Out>(),
            std::any::type_name::<T>()
        );

        // SAFETY: We know that the type of the data is the same as the type of the output.
        self.output_ch
            .send((
                item_id,
                data.map(|data| unsafe { transmute_unchecked(data) }),
            ))
            .map_err(|_| SendError::Closed)
    }
}

pub struct DynamicTaskContext {
    input_ch: Receiver<(ItemId, Box<dyn Any + Send + Sync>)>,
    output_ch: Sender<(ItemId, Result<Box<dyn Any + Send + Sync>>)>,
}

impl DynamicTaskContext {
    pub fn new(
        input_ch: Receiver<(ItemId, Box<dyn Any + Send + Sync>)>,
        output_ch: Sender<(ItemId, Result<Box<dyn Any + Send + Sync>>)>,
    ) -> Self {
        Self {
            input_ch,
            output_ch,
        }
    }
}

impl TaskContext for DynamicTaskContext {
    fn recv<T: Send + Sync + 'static + Clone>(&self) -> Result<(ItemId, T), ReceiveError> {
        let (id, data) = self.input_ch.recv()?;
        Ok((id, *data.downcast::<T>().unwrap()))
    }

    fn try_recv<T: Send + Sync + 'static + Clone>(
        &self,
    ) -> Result<Option<(ItemId, T)>, ReceiveError> {
        let result = self
            .input_ch
            .try_recv()?
            .map(|(id, data)| (id, *data.downcast::<T>().unwrap()));
        Ok(result)
    }

    fn send<T: Send + Sync + 'static>(
        &self,
        item_id: ItemId,
        data: Result<T>,
    ) -> Result<(), SendError> {
        self.output_ch.send((
            item_id,
            data.map(|data| Box::new(data) as Box<dyn Any + Send + Sync>),
        ))
    }
}

// https://github.com/funnsam/stable-intrinsics/blob/586a139ccb758488f109daf5165ecef574723b3e/src/lib.rs#L181
unsafe fn transmute_unchecked<Src, Dst>(src: Src) -> Dst {
    let dst = unsafe { mem::transmute::<*const Src, *const Dst>(&src as *const Src) };
    mem::forget(src);
    unsafe { dst.read() }
}
