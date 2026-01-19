use anyhow::Result;
use async_trait::async_trait;
use futures::{Stream, StreamExt, pin_mut};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::context::TaskContext;

pub trait Task<C>
where
    C: TaskContext,
{
    fn prepare(&mut self) -> Result<()>;

    fn clone_boxed(&self) -> Box<dyn Task<C> + Send + Sync>;

    fn run(&mut self, context: C) -> Result<()>;
}

pub trait SingleTask<C>: Task<C>
where
    C: TaskContext,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Serialize + Clone + Send + Sync + 'static;

    fn call(&mut self, input: Self::Input) -> Result<Self::Output>;

    fn run(&mut self, context: C) -> Result<()> {
        while let Ok((id, x)) = context.recv() {
            let y = self.call(x);
            let _ = context.send(id, y);
            let _ = context.send_end(id);
        }
        Ok(())
    }
}

pub trait BatchedTask<C>: Task<C>
where
    C: TaskContext,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Serialize + Clone + Send + Sync + 'static;

    fn batch_size(&self) -> usize;

    fn call(&mut self, input: Vec<Self::Input>) -> Vec<Result<Self::Output>>;

    fn run(&mut self, context: C) -> Result<()> {
        let batch_size = self.batch_size();
        let mut ids = Vec::with_capacity(batch_size);
        let mut buf = Vec::with_capacity(batch_size);

        while let Ok((id, x)) = context.recv() {
            ids.push(id);
            buf.push(x);

            if buf.len() >= batch_size {
                #[allow(clippy::drain_collect)]
                let ys = self.call(buf.drain(..).collect());
                debug_assert_eq!(ys.len(), batch_size);

                for (i, y) in ys.into_iter().enumerate() {
                    let _ = context.send(ids[i], y);
                    let _ = context.send_end(ids[i]);
                }
                ids.clear();
            }
        }

        if !buf.is_empty() {
            #[allow(clippy::drain_collect)]
            let ys = self.call(buf.drain(..).collect());
            for (i, y) in ys.into_iter().enumerate() {
                let _ = context.send(ids[i], y);
                let _ = context.send_end(ids[i]);
            }
        }
        Ok(())
    }
}

pub trait StreamTask<C>: Task<C>
where
    C: TaskContext,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Serialize + Clone + Send + Sync + 'static;

    fn call(&mut self, input: Self::Input) -> impl Iterator<Item = Result<Self::Output>>;

    fn run(&mut self, context: C) -> Result<()> {
        while let Ok((id, x)) = context.recv() {
            let ys = self.call(x);
            for y in ys {
                let _ = context.send(id, y);
            }
            let _ = context.send_end(id);
        }
        Ok(())
    }
}

#[async_trait]
pub trait AsyncTask<C>
where
    C: TaskContext + 'static,
{
    async fn prepare(&mut self) -> Result<()>;

    fn clone_box(&self) -> Box<dyn AsyncTask<C> + Send + Sync>;

    async fn run(&mut self, context: C) -> Result<()>;
}

#[async_trait]
pub trait AsyncSingleTask<C>: AsyncTask<C>
where
    C: TaskContext + 'static,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Serialize + Clone + Send + Sync + 'static;

    async fn call(&mut self, input: Self::Input) -> Result<Self::Output>;

    async fn run(&mut self, context: C) -> Result<()> {
        while let Ok((id, x)) = context.recv_async().await {
            let y = self.call(x).await;
            let _ = context.send_async(id, y).await;
            let _ = context.send_end_async(id).await;
        }
        Ok(())
    }
}

#[async_trait]
pub trait AsyncBatchedTask<C>: AsyncTask<C>
where
    C: TaskContext + 'static,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Serialize + Clone + Send + Sync + 'static;

    fn batch_size(&self) -> usize;

    async fn call(&mut self, input: Vec<Self::Input>) -> Vec<Result<Self::Output>>;

    async fn run(&mut self, context: C) -> Result<()> {
        let batch_size = self.batch_size();
        let mut ids = Vec::with_capacity(batch_size);
        let mut buf = Vec::with_capacity(batch_size);

        while let Ok((id, x)) = context.recv_async().await {
            ids.push(id);
            buf.push(x);

            if buf.len() >= batch_size {
                #[allow(clippy::drain_collect)]
                let ys = self.call(buf.drain(..).collect()).await;
                debug_assert_eq!(ys.len(), batch_size);

                for (i, y) in ys.into_iter().enumerate() {
                    let _ = context.send_async(ids[i], y).await;
                    let _ = context.send_end_async(ids[i]).await;
                }
                ids.clear();
            }
        }

        if !buf.is_empty() {
            #[allow(clippy::drain_collect)]
            let ys = self.call(buf.drain(..).collect()).await;
            for (i, y) in ys.into_iter().enumerate() {
                let _ = context.send_async(ids[i], y).await;
                let _ = context.send_end_async(ids[i]).await;
            }
        }
        Ok(())
    }
}

#[async_trait]
pub trait AsyncStreamTask<C>: AsyncTask<C>
where
    C: TaskContext + 'static,
{
    type Input: Send + DeserializeOwned + 'static;
    type Output: Serialize + Clone + Send + Sync + 'static;

    fn call(&mut self, input: Self::Input) -> impl Stream<Item = Result<Self::Output>> + Send;

    async fn run(&mut self, context: C) -> Result<()> {
        while let Ok((id, x)) = context.recv_async().await {
            let ys = self.call(x);
            pin_mut!(ys);
            while let Some(y) = ys.next().await {
                let _ = context.send_async(id, y).await;
            }
            let _ = context.send_end_async(id).await;
        }
        Ok(())
    }
}

impl<C> Clone for Box<dyn Task<C> + Send + Sync>
where
    C: TaskContext,
{
    fn clone(&self) -> Self {
        self.clone_boxed()
    }
}

impl<C> Clone for Box<dyn AsyncTask<C> + Send + Sync>
where
    C: TaskContext + 'static,
{
    fn clone(&self) -> Self {
        self.clone_box()
    }
}
