use std::fmt::{Display, Formatter};

use anyhow::Result;
use async_trait::async_trait;
use futures::{Stream, StreamExt, pin_mut};

pub use kanal::{Receiver, Sender};

mod context;
mod task;

type ItemId = usize;

#[derive(Debug, Copy, Clone)]
pub struct TaskEnd;

impl Display for TaskEnd {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "TaskEnd")
    }
}

impl std::error::Error for TaskEnd {}

pub trait Task {
    type Config;
    type Input: Send + Sync + Clone;
    type Output: Send + Sync + Clone;

    fn new(config: Self::Config) -> Self;

    fn prepare(&mut self) {}

    fn run(
        &mut self,
        recv: Receiver<(ItemId, Self::Input)>,
        send: Sender<(ItemId, Result<Self::Output>)>,
    );
}

pub trait SingleTask: Task {
    fn call(&mut self, input: Self::Input) -> Result<Self::Output>;

    fn run(
        &mut self,
        recv: Receiver<(ItemId, Self::Input)>,
        send: Sender<(ItemId, Result<Self::Output>)>,
    ) {
        while let Ok((id, x)) = recv.recv() {
            let y = self.call(x);
            let _ = send.send((id, y));
            // TODO: better way to signal end of task?
            let _ = send.send((id, Err(anyhow::Error::from(TaskEnd))));
        }
    }
}

pub trait BatchedTask: Task {
    fn batch_size(&self) -> usize;

    fn call(&mut self, input: Vec<Self::Input>) -> Vec<Result<Self::Output>>;

    fn run(
        &mut self,
        recv: Receiver<(ItemId, Self::Input)>,
        send: Sender<(ItemId, Result<Self::Output>)>,
    ) {
        let batch_size = self.batch_size();
        let mut ids = Vec::with_capacity(batch_size);
        let mut buf = Vec::with_capacity(batch_size);

        while let Ok((id, xs)) = recv.recv() {
            ids.push(id);
            buf.push(xs);

            if buf.len() >= batch_size {
                let ys = self.call(buf.drain(..).collect());
                debug_assert_eq!(ys.len(), batch_size);

                for (i, y) in ys.into_iter().enumerate() {
                    let _ = send.send((ids[i], y));
                    let _ = send.send((ids[i], Err(anyhow::Error::from(TaskEnd))));
                }
                ids.clear();
            }
        }

        if !buf.is_empty() {
            let ys = self.call(buf.drain(..).collect());
            for (i, y) in ys.into_iter().enumerate() {
                let _ = send.send((ids[i], y));
                let _ = send.send((ids[i], Err(anyhow::Error::from(TaskEnd))));
            }
        }
    }
}

pub trait StreamTask: Task {
    fn call(&mut self, input: Self::Input) -> impl Iterator<Item = Result<Self::Output>>;

    fn run(
        &mut self,
        recv: Receiver<(ItemId, Self::Input)>,
        send: Sender<(ItemId, Result<Self::Output>)>,
    ) {
        while let Ok((id, x)) = recv.recv() {
            let ys = self.call(x);
            for y in ys {
                let _ = send.send((id, y));
            }
            let _ = send.send((id, Err(anyhow::Error::from(TaskEnd))));
        }
    }
}

#[async_trait]
pub trait AsyncTask {
    type Config;
    type Input: Send + Sync + Clone;
    type Output: Send + Sync + Clone;

    fn new(config: Self::Config) -> Self;

    async fn prepare(&mut self) {}

    async fn run(
        &mut self,
        recv: Receiver<(ItemId, Self::Input)>,
        send: Sender<(ItemId, Result<Self::Output>)>,
    );
}

#[async_trait]
pub trait AsyncSingleTask: AsyncTask {
    async fn call(&mut self, input: Self::Input) -> Result<Self::Output>;

    async fn run(
        &mut self,
        recv: Receiver<(ItemId, Self::Input)>,
        send: Sender<(ItemId, Result<Self::Output>)>,
    ) {
        let recv = recv.to_async();
        let send = send.to_async();
        while let Ok((id, x)) = recv.recv().await {
            let y = self.call(x).await;
            let _ = send.send((id, y)).await;
            let _ = send.send((id, Err(anyhow::Error::from(TaskEnd)))).await;
        }
    }
}

#[async_trait]
pub trait AsyncBatchedTask: AsyncTask {
    fn batch_size(&self) -> usize;

    async fn call(&mut self, input: Vec<Self::Input>) -> Vec<Result<Self::Output>>;

    async fn run(
        &mut self,
        recv: Receiver<(ItemId, Self::Input)>,
        send: Sender<(ItemId, Result<Self::Output>)>,
    ) {
        let recv = recv.to_async();
        let send = send.to_async();
        let batch_size = self.batch_size();
        let mut ids = Vec::with_capacity(batch_size);
        let mut buf = Vec::with_capacity(batch_size);

        while let Ok((id, xs)) = recv.recv().await {
            ids.push(id);
            buf.push(xs);

            if buf.len() >= batch_size {
                let ys = self.call(buf.drain(..).collect()).await;
                debug_assert_eq!(ys.len(), batch_size);

                for (i, y) in ys.into_iter().enumerate() {
                    let _ = send.send((ids[i], y)).await;
                    let _ = send.send((ids[i], Err(anyhow::Error::from(TaskEnd)))).await;
                }
                ids.clear();
            }
        }

        if !buf.is_empty() {
            let ys = self.call(buf.drain(..).collect()).await;
            for (i, y) in ys.into_iter().enumerate() {
                let _ = send.send((ids[i], y)).await;
                let _ = send.send((ids[i], Err(anyhow::Error::from(TaskEnd)))).await;
            }
        }
    }
}

#[async_trait]
pub trait AsyncStreamTask: AsyncTask {
    fn call(&mut self, input: Self::Input) -> impl Stream<Item = Result<Self::Output>> + Send;

    async fn run(
        &mut self,
        recv: Receiver<(ItemId, Self::Input)>,
        send: Sender<(ItemId, Result<Self::Output>)>,
    ) {
        let recv = recv.to_async();
        let send = send.to_async();
        while let Ok((id, x)) = recv.recv().await {
            let ys = self.call(x);
            pin_mut!(ys);
            while let Some(y) = ys.next().await {
                let _ = send.send((id, y)).await;
            }
            let _ = send.send((id, Err(anyhow::Error::from(TaskEnd)))).await;
        }
    }
}
