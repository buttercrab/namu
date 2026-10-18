//! Workers on different machines, connected by TCP.
//!
//! Every worker listens on one address. A worker sending to a peer opens one connection to it
//! and keeps it, so messages to each peer arrive in order. Frames are a big-endian `u32` length
//! followed by the message as JSON; messages queued while a frame is written go out in the same
//! flush.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::Transport;
use crate::ids::WorkerId;
use crate::message::Message;

/// Frames larger than this are refused, so a corrupt length cannot exhaust memory.
const MAX_FRAME: u32 = 1 << 30;

/// How long a sender keeps trying to reach a peer that is not accepting connections yet.
const CONNECT_PATIENCE: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub struct TcpTransport {
    addrs: RwLock<HashMap<WorkerId, SocketAddr>>,
    links: Mutex<HashMap<WorkerId, mpsc::UnboundedSender<Message>>>,
}

impl TcpTransport {
    /// Listens on `addr`. Returns the transport, the address it listens on, and the inbox that
    /// receives every message sent to it.
    pub async fn bind(
        addr: SocketAddr,
    ) -> io::Result<(Arc<Self>, SocketAddr, mpsc::UnboundedReceiver<Message>)> {
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let (inbox, messages) = mpsc::unbounded_channel();
        tokio::spawn(accept(listener, inbox));
        Ok((Arc::new(Self::default()), local, messages))
    }

    /// Records where `worker` listens.
    pub fn set_peer(&self, worker: WorkerId, addr: SocketAddr) {
        let previous = self.addrs.write().unwrap().insert(worker, addr);
        if previous.is_some_and(|previous| previous != addr) {
            // It moved: the next message opens a new connection.
            self.links.lock().unwrap().remove(&worker);
        }
    }

    pub fn remove_peer(&self, worker: WorkerId) {
        self.addrs.write().unwrap().remove(&worker);
        self.links.lock().unwrap().remove(&worker);
    }

    fn link(&self, to: WorkerId) -> Option<mpsc::UnboundedSender<Message>> {
        let mut links = self.links.lock().unwrap();
        if let Some(link) = links.get(&to).filter(|link| !link.is_closed()) {
            return Some(link.clone());
        }
        let addr = *self.addrs.read().unwrap().get(&to)?;
        let (link, queue) = mpsc::unbounded_channel();
        tokio::spawn(write(to, addr, queue));
        links.insert(to, link.clone());
        Some(link)
    }
}

impl Transport for TcpTransport {
    fn send(&self, to: WorkerId, message: Message) {
        match self.link(to) {
            Some(link) if link.send(message).is_ok() => {}
            _ => tracing::error!(to, "message to an unknown or unreachable worker dropped"),
        }
    }
}

async fn accept(listener: TcpListener, inbox: mpsc::UnboundedSender<Message>) {
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let _ = stream.set_nodelay(true);
                tokio::spawn(read(stream, peer, inbox.clone()));
            }
            Err(err) => tracing::error!("accept failed: {err}"),
        }
    }
}

async fn read(stream: TcpStream, peer: SocketAddr, inbox: mpsc::UnboundedSender<Message>) {
    let mut stream = BufReader::new(stream);
    let mut frame = Vec::new();
    loop {
        let len = match stream.read_u32().await {
            Ok(len) if len <= MAX_FRAME => len as usize,
            Ok(len) => return tracing::error!(%peer, len, "oversized frame; closing"),
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return,
            Err(err) => return tracing::error!(%peer, "read failed: {err}"),
        };
        frame.resize(len, 0);
        if let Err(err) = stream.read_exact(&mut frame).await {
            return tracing::error!(%peer, "read failed: {err}");
        }
        match serde_json::from_slice(&frame) {
            Ok(message) => {
                if inbox.send(message).is_err() {
                    return;
                }
            }
            Err(err) => tracing::error!(%peer, "undecodable message dropped: {err}"),
        }
    }
}

async fn write(to: WorkerId, addr: SocketAddr, mut queue: mpsc::UnboundedReceiver<Message>) {
    let Some(stream) = connect(addr).await else {
        tracing::error!(to, %addr, "could not connect; dropping its messages");
        return;
    };
    let mut stream = BufWriter::new(stream);
    let mut frame = Vec::new();
    while let Some(message) = queue.recv().await {
        let mut next = Some(message);
        // Write everything queued so far, then flush once.
        while let Some(message) = next {
            frame.clear();
            frame.extend_from_slice(&[0; 4]);
            if let Err(err) = serde_json::to_writer(&mut frame, &message) {
                tracing::error!(to, "unencodable message dropped: {err}");
            } else {
                let len = (frame.len() - 4) as u32;
                frame[..4].copy_from_slice(&len.to_be_bytes());
                if let Err(err) = stream.write_all(&frame).await {
                    return tracing::error!(to, "write failed: {err}");
                }
            }
            next = queue.try_recv().ok();
        }
        if let Err(err) = stream.flush().await {
            return tracing::error!(to, "write failed: {err}");
        }
    }
}

async fn connect(addr: SocketAddr) -> Option<TcpStream> {
    let deadline = tokio::time::Instant::now() + CONNECT_PATIENCE;
    let mut backoff = Duration::from_millis(10);
    loop {
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                return Some(stream);
            }
            Err(_) if tokio::time::Instant::now() + backoff < deadline => {
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(1));
            }
            Err(_) => return None,
        }
    }
}
