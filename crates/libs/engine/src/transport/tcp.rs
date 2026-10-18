//! Workers on different machines, connected by TCP.
//!
//! Every worker listens on one address. A worker sending to a peer opens one connection to it
//! and keeps it, so messages to each peer arrive in order. Frames are a big-endian `u32` length
//! followed by the message as JSON; messages queued while a frame is written go out in the same
//! flush.
//!
//! A worker can hear from a peer before it learns where the peer listens: a worker that just
//! joined is in the membership the orchestrator gives it, but its peers learn of it only at their
//! next heartbeat. Messages to such a peer wait in its queue, and go out once its address arrives.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
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

type Queue = mpsc::UnboundedReceiver<Message>;

#[derive(Debug, Default)]
pub struct TcpTransport {
    peers: Mutex<Peers>,
}

#[derive(Debug, Default)]
struct Peers {
    addrs: HashMap<WorkerId, SocketAddr>,
    /// The queue of messages to each peer, written out by its connection.
    links: HashMap<WorkerId, mpsc::UnboundedSender<Message>>,
    /// The queues of peers whose address is not known yet: no connection writes them out until
    /// it is.
    unplaced: HashMap<WorkerId, Queue>,
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
        let mut peers = self.peers.lock().unwrap();
        let previous = peers.addrs.insert(worker, addr);
        if previous.is_some_and(|previous| previous != addr) {
            // It moved: the next message opens a new connection.
            peers.links.remove(&worker);
        }
        if let Some(queue) = peers.unplaced.remove(&worker) {
            tokio::spawn(write(worker, addr, queue));
        }
    }

    pub fn remove_peer(&self, worker: WorkerId) {
        let mut peers = self.peers.lock().unwrap();
        peers.addrs.remove(&worker);
        peers.links.remove(&worker);
        peers.unplaced.remove(&worker);
    }

    fn link(&self, to: WorkerId) -> mpsc::UnboundedSender<Message> {
        let mut peers = self.peers.lock().unwrap();
        if let Some(link) = peers.links.get(&to).filter(|link| !link.is_closed()) {
            return link.clone();
        }
        let (link, queue) = mpsc::unbounded_channel();
        match peers.addrs.get(&to) {
            Some(&addr) => {
                tokio::spawn(write(to, addr, queue));
            }
            None => {
                peers.unplaced.insert(to, queue);
            }
        }
        peers.links.insert(to, link.clone());
        link
    }
}

impl Transport for TcpTransport {
    fn send(&self, to: WorkerId, message: Message) {
        if self.link(to).send(message).is_err() {
            tracing::error!(to, "message to an unreachable worker dropped");
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

async fn write(to: WorkerId, addr: SocketAddr, mut queue: Queue) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn messages_wait_for_the_address_of_their_peer() {
        let any = "127.0.0.1:0".parse().unwrap();
        let (sender, _, _) = TcpTransport::bind(any).await.unwrap();
        let (_, addr, mut inbox) = TcpTransport::bind(any).await.unwrap();
        let run = uuid::Uuid::new_v4();
        sender.send(7, Message::RunOver { run });
        sender.set_peer(7, addr);
        let message = tokio::time::timeout(Duration::from_secs(5), inbox.recv()).await;
        assert!(matches!(message, Ok(Some(Message::RunOver { run: got })) if got == run));
    }
}
