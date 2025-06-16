use crate::context::TaskContext;
use anyhow::Result;
use async_trait::async_trait;
use futures::{Stream, StreamExt, pin_mut};
use serde::{de::DeserializeOwned, Serialize};

pub trait Task<Id> {
    fn prepare(&mut self) -> Result<()>;

    fn run<C: TaskContext<Id>>(&mut self, context: C) -> Result<()>;
}

pub trait SingleTask<Id>: Task<Id>
where
    Id: Clone,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Send + Serialize + 'static;

    fn call(&mut self, input: Self::Input) -> Result<Self::Output>;

    fn run<C: TaskContext<Id>>(&mut self, context: C) -> Result<()> {
        while let Ok((id, x)) = context.recv() {
            let y = self.call(x);
            let _ = context.send(id.clone(), y);
            let _ = context.send_end(id);
        }
        Ok(())
    }
}

pub trait BatchedTask<Id>: Task<Id>
where
    Id: Clone,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Send + Serialize + 'static;

    fn batch_size(&self) -> usize;

    fn call(&mut self, input: Vec<Self::Input>) -> Vec<Result<Self::Output>>;

    fn run<C: TaskContext<Id>>(&mut self, context: C) -> Result<()> {
        let batch_size = self.batch_size();
        let mut ids = Vec::with_capacity(batch_size);
        let mut buf = Vec::with_capacity(batch_size);

        while let Ok((id, x)) = context.recv() {
            ids.push(id.clone());
            buf.push(x);

            if buf.len() >= batch_size {
                let ys = self.call(buf.drain(..).collect());
                debug_assert_eq!(ys.len(), batch_size);

                for (i, y) in ys.into_iter().enumerate() {
                    let _ = context.send(ids[i].clone(), y);
                    let _ = context.send_end(ids[i].clone());
                }
                ids.clear();
            }
        }

        if !buf.is_empty() {
            let ys = self.call(buf.drain(..).collect());
            for (i, y) in ys.into_iter().enumerate() {
                let _ = context.send(ids[i].clone(), y);
                let _ = context.send_end(ids[i].clone());
            }
        }
        Ok(())
    }
}

pub trait StreamTask<Id>: Task<Id>
where
    Id: Clone,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Send + Serialize + 'static;

    fn call(&mut self, input: Self::Input) -> impl Iterator<Item = Result<Self::Output>>;

    fn run<C: TaskContext<Id>>(&mut self, context: C) -> Result<()> {
        while let Ok((id, x)) = context.recv() {
            let ys = self.call(x);
            for y in ys {
                let _ = context.send(id.clone(), y);
            }
            let _ = context.send_end(id);
        }
        Ok(())
    }
}

#[async_trait]
pub trait AsyncTask<Id> {
    async fn prepare(&mut self) -> Result<()>;

    async fn run<C: TaskContext<Id>>(&mut self, context: C) -> Result<()>;
}

#[async_trait]
pub trait AsyncSingleTask<Id>: AsyncTask<Id>
where
    Id: Clone + Send,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Send + Serialize + 'static;

    async fn call(&mut self, input: Self::Input) -> Result<Self::Output>;

    async fn run<C: TaskContext<Id>>(&mut self, context: C) -> Result<()> {
        while let Ok((id, x)) = context.recv_async().await {
            let y = self.call(x).await;
            let _ = context.send_async(id.clone(), y).await;
            let _ = context.send_end_async(id).await;
        }
        Ok(())
    }
}

#[async_trait]
pub trait AsyncBatchedTask<Id>: AsyncTask<Id>
where
    Id: Clone + Send,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Send + Serialize + 'static;

    fn batch_size(&self) -> usize;

    async fn call(&mut self, input: Vec<Self::Input>) -> Vec<Result<Self::Output>>;

    async fn run<C: TaskContext<Id>>(&mut self, context: C) -> Result<()> {
        let batch_size = self.batch_size();
        let mut ids = Vec::with_capacity(batch_size);
        let mut buf = Vec::with_capacity(batch_size);

        while let Ok((id, x)) = context.recv_async().await {
            ids.push(id.clone());
            buf.push(x);

            if buf.len() >= batch_size {
                let ys = self.call(buf.drain(..).collect()).await;
                debug_assert_eq!(ys.len(), batch_size);

                for (i, y) in ys.into_iter().enumerate() {
                    let _ = context.send_async(ids[i].clone(), y).await;
                    let _ = context.send_end_async(ids[i].clone()).await;
                }
                ids.clear();
            }
        }

        if !buf.is_empty() {
            let ys = self.call(buf.drain(..).collect()).await;
            for (i, y) in ys.into_iter().enumerate() {
                let _ = context.send_async(ids[i].clone(), y).await;
                let _ = context.send_end_async(ids[i].clone()).await;
            }
        }
        Ok(())
    }
}

#[async_trait]
pub trait AsyncStreamTask<Id>: AsyncTask<Id>
where
    Id: Clone + Send,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Send + Serialize + 'static;

    fn call(&mut self, input: Self::Input) -> impl Stream<Item = Result<Self::Output>> + Send;

    async fn run<C: TaskContext<Id>>(&mut self, context: C) -> Result<()> {
        while let Ok((id, x)) = context.recv_async().await {
            let ys = self.call(x);
            pin_mut!(ys);
            while let Some(y) = ys.next().await {
                let _ = context.send_async(id.clone(), y).await;
            }
            let _ = context.send_end_async(id).await;
        }
        Ok(())
    }
}
