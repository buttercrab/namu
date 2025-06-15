use crate::context::TaskContext;
use anyhow::Result;
use async_trait::async_trait;
use futures::{Stream, StreamExt, pin_mut};

pub trait Task<Id, C>
where
    C: TaskContext<Id>,
{
    fn prepare(&mut self) -> Result<()>;

    fn run(&mut self, context: C) -> Result<()>;
}

pub trait SingleTask<Id, C>: Task<Id, C>
where
    Id: Clone,
    C: TaskContext<Id>,
{
    type Input: Send + Sync + 'static;
    type Output: Send + Sync + 'static;

    fn call(&mut self, input: Self::Input) -> Result<Self::Output>;

    fn run(&mut self, context: C) {
        while let Ok((id, x)) = context.recv() {
            let y = self.call(x);
            let _ = context.send(id.clone(), y);
            let _ = context.send_end(id);
        }
    }
}

pub trait BatchedTask<Id, C>: Task<Id, C>
where
    Id: Clone,
    C: TaskContext<Id>,
{
    type Input: Send + Sync + 'static;
    type Output: Send + Sync + 'static;

    fn batch_size(&self) -> usize;

    fn call(&mut self, input: Vec<Self::Input>) -> Vec<Result<Self::Output>>;

    fn run(&mut self, context: C) {
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
    }
}

pub trait StreamTask<Id, C>: Task<Id, C>
where
    Id: Clone,
    C: TaskContext<Id>,
{
    type Input: Send + Sync + 'static;
    type Output: Send + Sync + 'static;

    fn call(&mut self, input: Self::Input) -> impl Iterator<Item = Result<Self::Output>>;

    fn run(&mut self, context: C) {
        while let Ok((id, x)) = context.recv() {
            let ys = self.call(x);
            for y in ys {
                let _ = context.send(id.clone(), y);
            }
            let _ = context.send_end(id);
        }
    }
}

#[async_trait]
pub trait AsyncTask<Id, C> {
    async fn prepare(&mut self) -> Result<()>;

    async fn run(&mut self, context: C) -> Result<()>;
}

#[async_trait]
pub trait AsyncSingleTask<Id, C>: AsyncTask<Id, C>
where
    Id: Clone + Send,
    C: TaskContext<Id> + 'static,
{
    type Input: Send + 'static;
    type Output: Send + 'static;

    async fn call(&mut self, input: Self::Input) -> Result<Self::Output>;

    async fn run(&mut self, context: C) {
        while let Ok((id, x)) = context.recv_async().await {
            let y = self.call(x).await;
            let _ = context.send_async(id.clone(), y).await;
            let _ = context.send_end_async(id).await;
        }
    }
}

#[async_trait]
pub trait AsyncBatchedTask<Id, C>: AsyncTask<Id, C>
where
    Id: Clone + Send,
    C: TaskContext<Id> + 'static,
{
    type Input: Send + 'static;
    type Output: Send + 'static;

    fn batch_size(&self) -> usize;

    async fn call(&mut self, input: Vec<Self::Input>) -> Vec<Result<Self::Output>>;

    async fn run(&mut self, context: C) {
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
    }
}

#[async_trait]
pub trait AsyncStreamTask<Id, C>: AsyncTask<Id, C>
where
    Id: Clone + Send,
    C: TaskContext<Id> + 'static,
{
    type Input: Send + 'static;
    type Output: Send + 'static;

    fn call(&mut self, input: Self::Input) -> impl Stream<Item = Result<Self::Output>> + Send;

    async fn run(&mut self, context: C) {
        while let Ok((id, x)) = context.recv_async().await {
            let ys = self.call(x);
            pin_mut!(ys);
            while let Some(y) = ys.next().await {
                let _ = context.send_async(id.clone(), y).await;
            }
            let _ = context.send_end_async(id).await;
        }
    }
}
