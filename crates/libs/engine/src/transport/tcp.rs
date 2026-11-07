//! Workers on different machines, connected by TCP.
//!
//! Every worker listens on one address. Messages to a peer form a **session**: each gets the next
//! sequence number and is kept until the peer acknowledges it, so a broken connection loses
//! nothing. The sender reconnects and replays what is unacknowledged, and the receiver delivers
//! each sequence number once. Messages to a peer arrive exactly once and in order for as long as
//! both live; a peer that dies is the business of recovery (design §7), not of the transport.
//!
//! A sender opens one connection per peer. It starts with the session's id, then carries frames:
//! a sequence number, a big-endian `u32` length, and the message as MessagePack. Messages queued
//! while a frame is written go out in the same flush. The receiver answers on the same connection
//! with the number of messages of the session it has delivered.
//!
//! With [`MeshTls`], every connection is mutual TLS: each side proves it is a member of the
//! cluster before a byte of a session crosses (see `tls`). Without it, connections are plain TCP,
//! for a cluster on one trusted machine.
//!
//! A receiver that restarts (the orchestrator, which keeps its id) has forgotten its sessions; the
//! next connection of each takes up where its sender stands, and what was delivered but not yet
//! acknowledged before the restart comes again.
//!
//! A worker can hear from a peer before it learns where the peer listens: a worker that just
//! joined is in the membership the orchestrator gives it, but its peers learn of it only at their
//! next heartbeat. Messages to such a peer wait in its session until its address arrives.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadHalf, WriteHalf,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

use super::Transport;
use super::tls::MeshTls;
use crate::ids::WorkerId;
use crate::message::Message;

/// Frames larger than this are refused, so a corrupt length cannot exhaust memory.
const MAX_FRAME: u32 = 1 << 30;

/// The longest wait between attempts to reach a peer.
const MAX_BACKOFF: Duration = Duration::from_secs(1);

type SessionId = u64;

/// A connection: TCP, or TLS over it.
trait Stream: AsyncRead + AsyncWrite + Send + Unpin {}

impl<S: AsyncRead + AsyncWrite + Send + Unpin> Stream for S {}

type Connection = Box<dyn Stream>;

#[derive(Debug, Default)]
pub struct TcpTransport {
    peers: Mutex<Peers>,
    /// How many messages of each session this transport has delivered.
    delivered: Arc<Mutex<HashMap<SessionId, u64>>>,
}

#[derive(Debug, Default)]
struct Peers {
    tls: Option<MeshTls>,
    sessions: HashMap<WorkerId, Session>,
    /// Peers that left the membership. They never return: a worker that rejoins has a new id.
    gone: HashSet<WorkerId>,
}

/// The sending side of a session: its queue, and where the peer listens.
#[derive(Debug)]
struct Session {
    queue: mpsc::UnboundedSender<Message>,
    place: watch::Sender<Place>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Unknown,
    At(SocketAddr),
    Gone,
}

impl TcpTransport {
    /// Listens on `addr`, with mutual TLS if `tls` is given. Returns the transport, the address
    /// it listens on, and the inbox that receives every message sent to it.
    pub async fn bind(
        addr: SocketAddr,
        tls: Option<MeshTls>,
    ) -> io::Result<(Arc<Self>, SocketAddr, mpsc::UnboundedReceiver<Message>)> {
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let (inbox, messages) = mpsc::unbounded_channel();
        let transport = Arc::new(Self::default());
        transport.peers.lock().unwrap().tls = tls.clone();
        tokio::spawn(accept(listener, tls, inbox, transport.delivered.clone()));
        Ok((transport, local, messages))
    }

    /// Records where `worker` listens.
    pub fn set_peer(&self, worker: WorkerId, addr: SocketAddr) {
        let mut peers = self.peers.lock().unwrap();
        if let Some(session) = peers.session(worker) {
            session.place.send_if_modified(|place| {
                let moved = *place != Place::At(addr);
                *place = Place::At(addr);
                moved
            });
        }
    }

    /// Makes the peers that have an address exactly `members`: the others have left, and what
    /// was still unacknowledged to them is dropped. Peers whose address is not known yet keep
    /// waiting for it.
    pub fn set_peers(&self, members: &[(WorkerId, SocketAddr)]) {
        for &(worker, addr) in members {
            self.set_peer(worker, addr);
        }
        let mut peers = self.peers.lock().unwrap();
        let left: Vec<WorkerId> = peers
            .sessions
            .iter()
            .filter(|(worker, session)| {
                *session.place.borrow() != Place::Unknown
                    && !members.iter().any(|(member, _)| member == *worker)
            })
            .map(|(worker, _)| *worker)
            .collect();
        for worker in left {
            peers.remove(worker);
        }
    }

    /// `worker` has left: what was still unacknowledged to it is dropped.
    pub fn remove_peer(&self, worker: WorkerId) {
        self.peers.lock().unwrap().remove(worker);
    }
}

impl Peers {
    /// The session to `worker`, started on first use. `None` if the worker has left.
    fn session(&mut self, worker: WorkerId) -> Option<&Session> {
        if self.gone.contains(&worker) {
            return None;
        }
        let tls = self.tls.clone();
        Some(self.sessions.entry(worker).or_insert_with(|| {
            let (queue, messages) = mpsc::unbounded_channel();
            let (place, placed) = watch::channel(Place::Unknown);
            tokio::spawn(run_session(worker, tls, placed, messages));
            Session { queue, place }
        }))
    }

    fn remove(&mut self, worker: WorkerId) {
        if let Some(session) = self.sessions.remove(&worker) {
            session.place.send_replace(Place::Gone);
        }
        self.gone.insert(worker);
    }
}

impl Transport for TcpTransport {
    fn send(&self, to: WorkerId, message: Message) {
        let mut peers = self.peers.lock().unwrap();
        match peers.session(to) {
            Some(session) => {
                // The session ends only when it is removed, which removes it from the map too.
                let _ = session.queue.send(message);
            }
            None => tracing::debug!(to, "message to a worker that left dropped"),
        }
    }
}

/// Runs the sending side of a session until its peer leaves.
async fn run_session(
    to: WorkerId,
    tls: Option<MeshTls>,
    mut place: watch::Receiver<Place>,
    mut queue: mpsc::UnboundedReceiver<Message>,
) {
    let session: SessionId = uuid::Uuid::new_v4().as_u64_pair().0;
    let mut outgoing = Outgoing::default();
    while let Some(stream) = connect(tls.as_ref(), &mut place).await {
        let (read, write) = tokio::io::split(stream);
        let (acks, mut acked) = mpsc::unbounded_channel();
        let reader = tokio::spawn(read_acks(read, acks));
        let mut link = Link {
            write: BufWriter::new(write),
            acked: &mut acked,
        };
        let result = link
            .run(session, &mut outgoing, &mut queue, &mut place)
            .await;
        reader.abort();
        match result {
            Ok(Ended::Closed) => return,
            Ok(Ended::Moved) => {}
            Err(err) => tracing::warn!(to, "connection lost, reconnecting: {err}"),
        }
    }
}

/// Messages sent in a session and not yet acknowledged, as encoded frames.
#[derive(Default)]
struct Outgoing {
    next: u64,
    unacked: VecDeque<(u64, Vec<u8>)>,
}

impl Outgoing {
    /// Numbers and encodes `message`, and keeps it until acknowledged.
    fn push(&mut self, message: &Message) -> Option<&[u8]> {
        let mut frame = Vec::new();
        frame.extend_from_slice(&self.next.to_be_bytes());
        frame.extend_from_slice(&[0; 4]);
        if let Err(err) = rmp_serde::encode::write_named(&mut frame, message) {
            tracing::error!("unencodable message dropped: {err}");
            return None;
        }
        let len = (frame.len() - 12) as u32;
        frame[8..12].copy_from_slice(&len.to_be_bytes());
        self.unacked.push_back((self.next, frame));
        self.next += 1;
        self.unacked.back().map(|(_, frame)| frame.as_slice())
    }

    /// The peer has delivered every message numbered below `delivered`.
    fn ack(&mut self, delivered: u64) {
        while self
            .unacked
            .front()
            .is_some_and(|(seq, _)| *seq < delivered)
        {
            self.unacked.pop_front();
        }
    }
}

enum Ended {
    /// The session's peer left.
    Closed,
    /// The peer's address changed.
    Moved,
}

/// One connection of a session.
struct Link<'a> {
    write: BufWriter<WriteHalf<Connection>>,
    acked: &'a mut mpsc::UnboundedReceiver<u64>,
}

impl Link<'_> {
    async fn run(
        &mut self,
        session: SessionId,
        outgoing: &mut Outgoing,
        queue: &mut mpsc::UnboundedReceiver<Message>,
        place: &mut watch::Receiver<Place>,
    ) -> io::Result<Ended> {
        self.write.write_u64(session).await?;
        for (_, frame) in &outgoing.unacked {
            self.write.write_all(frame).await?;
        }
        self.write.flush().await?;
        loop {
            tokio::select! {
                message = queue.recv() => {
                    let Some(message) = message else { return Ok(Ended::Closed) };
                    // Write everything queued so far, then flush once.
                    let mut next = Some(message);
                    while let Some(message) = next {
                        if let Some(frame) = outgoing.push(&message) {
                            self.write.write_all(frame).await?;
                        }
                        next = queue.try_recv().ok();
                    }
                    self.write.flush().await?;
                }
                delivered = self.acked.recv() => match delivered {
                    Some(delivered) => outgoing.ack(delivered),
                    None => return Err(io::ErrorKind::ConnectionReset.into()),
                },
                _ = place.changed() => return Ok(Ended::Moved),
            }
        }
    }
}

async fn read_acks(read: ReadHalf<Connection>, acks: mpsc::UnboundedSender<u64>) {
    let mut read = BufReader::new(read);
    while let Ok(delivered) = read.read_u64().await {
        if acks.send(delivered).is_err() {
            return;
        }
    }
}

/// Connects to the peer once its address is known, retrying until it accepts. `None` once the
/// peer has left.
async fn connect(tls: Option<&MeshTls>, place: &mut watch::Receiver<Place>) -> Option<Connection> {
    let mut backoff = Duration::from_millis(10);
    loop {
        let current = *place.borrow_and_update();
        match current {
            Place::Gone => return None,
            Place::Unknown => {}
            Place::At(addr) => match open(tls, addr).await {
                Ok(stream) => return Some(stream),
                Err(err) => {
                    if err.kind() == io::ErrorKind::InvalidData {
                        tracing::warn!(%addr, "TLS handshake failed: {err}");
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        _ = place.changed() => {}
                    }
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    continue;
                }
            },
        }
        if place.changed().await.is_err() {
            return None;
        }
    }
}

/// A connection to `addr`: TCP, then TLS if the mesh has it.
async fn open(tls: Option<&MeshTls>, addr: SocketAddr) -> io::Result<Connection> {
    let stream = TcpStream::connect(addr).await?;
    let _ = stream.set_nodelay(true);
    Ok(match tls {
        Some(tls) => Box::new(
            tls.connector()
                .connect(MeshTls::server_name(), stream)
                .await?,
        ),
        None => Box::new(stream),
    })
}

async fn accept(
    listener: TcpListener,
    tls: Option<MeshTls>,
    inbox: mpsc::UnboundedSender<Message>,
    delivered: Arc<Mutex<HashMap<SessionId, u64>>>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let _ = stream.set_nodelay(true);
                let (tls, inbox, delivered) = (tls.clone(), inbox.clone(), delivered.clone());
                // The handshake runs here, not in the loop: a slow or hostile peer holds up only
                // its own connection.
                tokio::spawn(async move {
                    let stream: Connection = match tls {
                        Some(tls) => match tls.acceptor().accept(stream).await {
                            Ok(stream) => Box::new(stream),
                            Err(err) => {
                                return tracing::warn!(%peer, "refused a connection: {err}");
                            }
                        },
                        None => Box::new(stream),
                    };
                    if let Err(err) = receive(stream, inbox, delivered).await
                        && err.kind() != io::ErrorKind::UnexpectedEof
                    {
                        tracing::warn!(%peer, "connection closed: {err}");
                    }
                });
            }
            Err(err) => tracing::error!("accept failed: {err}"),
        }
    }
}

/// Receives one connection of a session: delivers each message the session has not delivered
/// yet, and acknowledges what it has.
async fn receive(
    stream: Connection,
    inbox: mpsc::UnboundedSender<Message>,
    delivered: Arc<Mutex<HashMap<SessionId, u64>>>,
) -> io::Result<()> {
    let (read, write) = tokio::io::split(stream);
    let (mut read, mut write) = (BufReader::new(read), BufWriter::new(write));
    let session = read.read_u64().await?;
    let mut frame = Vec::new();
    loop {
        let seq = read.read_u64().await?;
        let len = read.read_u32().await?;
        if len > MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "oversized frame",
            ));
        }
        frame.resize(len as usize, 0);
        read.read_exact(&mut frame).await?;
        let message = rmp_serde::from_slice::<Message>(&frame);
        let count = {
            let mut delivered = delivered.lock().unwrap();
            // A session this receiver has never seen starts where its sender stands: at 0, or,
            // if this receiver restarted, past what its last life acknowledged. Then messages it
            // delivered but did not acknowledge before it died come again: across a restart, a
            // session is at least once.
            let count = delivered.entry(session).or_insert(seq);
            if seq == *count {
                // Delivered under the lock, so two connections of one session cannot reorder.
                match message {
                    Ok(message) => {
                        if inbox.send(message).is_err() {
                            return Ok(());
                        }
                    }
                    Err(err) => tracing::error!("undecodable message dropped: {err}"),
                }
                *count += 1;
            } else if seq > *count {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a gap in the session",
                ));
            }
            *count
        };
        write.write_u64(count).await?;
        if read.buffer().is_empty() {
            write.flush().await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::RunId;

    fn any() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    async fn received(inbox: &mut mpsc::UnboundedReceiver<Message>) -> Option<RunId> {
        match tokio::time::timeout(Duration::from_secs(2), inbox.recv()).await {
            Ok(Some(Message::RunOver { run })) => Some(run),
            _ => None,
        }
    }

    #[tokio::test]
    async fn messages_wait_for_the_address_of_their_peer() {
        let (sender, _, _) = TcpTransport::bind(any(), None).await.unwrap();
        let (_, addr, mut inbox) = TcpTransport::bind(any(), None).await.unwrap();
        let run = uuid::Uuid::new_v4();
        sender.send(7, Message::RunOver { run });
        sender.set_peer(7, addr);
        assert_eq!(received(&mut inbox).await, Some(run));
    }

    /// Forwards connections to `to`, and can cut every connection it carries.
    struct Proxy {
        addr: SocketAddr,
        cut: watch::Sender<u64>,
    }

    impl Proxy {
        async fn start(to: SocketAddr) -> Self {
            let listener = TcpListener::bind(any()).await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (cut, cuts) = watch::channel(0);
            tokio::spawn(async move {
                while let Ok((client, _)) = listener.accept().await {
                    let server = TcpStream::connect(to).await.unwrap();
                    let mut cuts = cuts.clone();
                    cuts.borrow_and_update();
                    tokio::spawn(async move {
                        let (mut client, mut server) = (client, server);
                        tokio::select! {
                            _ = tokio::io::copy_bidirectional(&mut client, &mut server) => {}
                            _ = cuts.changed() => {}
                        }
                    });
                }
            });
            Self { addr, cut }
        }

        fn cut(&self) {
            self.cut.send_modify(|cuts| *cuts += 1);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_cut_connection_loses_and_repeats_nothing() {
        let (sender, _, _) = TcpTransport::bind(any(), None).await.unwrap();
        let (_, addr, mut inbox) = TcpTransport::bind(any(), None).await.unwrap();
        let proxy = Proxy::start(addr).await;
        sender.set_peer(1, proxy.addr);
        let runs: Vec<RunId> = (0..500).map(|_| uuid::Uuid::new_v4()).collect();
        let mut got = Vec::new();
        for batch in runs.chunks(100) {
            for &run in batch {
                sender.send(1, Message::RunOver { run });
            }
            // Cut while the rest of the batch is in flight.
            got.push(received(&mut inbox).await);
            proxy.cut();
        }
        while got.len() < runs.len()
            && let Some(run) = received(&mut inbox).await
        {
            got.push(Some(run));
        }
        assert_eq!(got.len(), runs.len(), "messages were lost");
        assert!(
            got.into_iter().eq(runs.into_iter().map(Some)),
            "messages were reordered"
        );
        let more = tokio::time::timeout(Duration::from_millis(200), inbox.recv()).await;
        assert!(more.is_err(), "a message arrived twice");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn members_of_a_cluster_talk_over_mutual_tls() {
        let ca = crate::transport::certs::ClusterCa::new().unwrap();
        let (sender, _, _) = TcpTransport::bind(any(), Some(ca.mesh_tls().unwrap()))
            .await
            .unwrap();
        let (_, addr, mut inbox) = TcpTransport::bind(any(), Some(ca.mesh_tls().unwrap()))
            .await
            .unwrap();
        // Through the proxy too: a cut TLS connection is replaced like a plain one.
        let proxy = Proxy::start(addr).await;
        sender.set_peer(1, proxy.addr);
        let runs: Vec<RunId> = (0..50).map(|_| uuid::Uuid::new_v4()).collect();
        for &run in &runs[..25] {
            sender.send(1, Message::RunOver { run });
        }
        assert_eq!(received(&mut inbox).await, Some(runs[0]));
        proxy.cut();
        for &run in &runs[25..] {
            sender.send(1, Message::RunOver { run });
        }
        let mut got = vec![runs[0]];
        while let Some(run) = received(&mut inbox).await {
            got.push(run);
            if got.len() == runs.len() {
                break;
            }
        }
        assert_eq!(got, runs);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn strangers_cannot_reach_a_member() {
        let ca = crate::transport::certs::ClusterCa::new().unwrap();
        let other = crate::transport::certs::ClusterCa::new().unwrap();
        let (_, addr, mut inbox) = TcpTransport::bind(any(), Some(ca.mesh_tls().unwrap()))
            .await
            .unwrap();

        // A member of another cluster: its certificate does not chain to this one's CA.
        let (stranger, _, _) = TcpTransport::bind(any(), Some(other.mesh_tls().unwrap()))
            .await
            .unwrap();
        stranger.set_peer(1, addr);
        stranger.send(
            1,
            Message::RunOver {
                run: uuid::Uuid::new_v4(),
            },
        );

        // Plain TCP, as the mesh was before.
        let (plain, _, _) = TcpTransport::bind(any(), None).await.unwrap();
        plain.set_peer(1, addr);
        plain.send(
            1,
            Message::RunOver {
                run: uuid::Uuid::new_v4(),
            },
        );

        let arrived = tokio::time::timeout(Duration::from_millis(500), inbox.recv()).await;
        assert!(
            arrived.is_err(),
            "a stranger's message arrived: {arrived:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_receiver_takes_up_a_session_where_it_stands() {
        let (sender, _, _) = TcpTransport::bind(any(), None).await.unwrap();
        let (receiver, addr, mut inbox) = TcpTransport::bind(any(), None).await.unwrap();
        let proxy = Proxy::start(addr).await;
        sender.set_peer(1, proxy.addr);
        let first = uuid::Uuid::new_v4();
        sender.send(1, Message::RunOver { run: first });
        assert_eq!(received(&mut inbox).await, Some(first));

        // The receiver dies: its connections go, and with them what it knew of the session.
        drop((receiver, inbox));
        proxy.cut();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let later: Vec<RunId> = (0..20).map(|_| uuid::Uuid::new_v4()).collect();
        for &run in &later {
            sender.send(1, Message::RunOver { run });
        }

        // A new receiver, where the old one was reached: everything sent since arrives, in order.
        let (_, moved, mut inbox) = TcpTransport::bind(any(), None).await.unwrap();
        sender.set_peer(1, moved);
        let mut got = Vec::new();
        while got.len() < later.len()
            && let Some(run) = received(&mut inbox).await
        {
            got.push(run);
        }
        assert_eq!(got, later);
    }
}
