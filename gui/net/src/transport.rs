//! [`nds_core::nds::MpTransport`] implementation carrying MP frames over a
//! UDP socket between `lunaris` processes. See
//! `docs/design/design_lan.md` §8.2.
//!
//! A dedicated background thread owns the receive side (blocking
//! `recv_from` in a loop) and classifies each datagram by
//! [`crate::wire::WireFrameKind`] into one of two channels: "regular"
//! (packet/cmd/ack -- consumed by [`NetTransport::recv_packet`] and
//! [`NetTransport::recv_host_packet`]) or "reply" (consumed only by
//! [`NetTransport::recv_replies`]). Splitting them avoids the two callers
//! racing to read the same frame off one channel.

use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{Receiver, Sender, TryRecvError},
    },
    time::Duration,
};

use nds_core::nds::{LinkHints, MpFrameKind, MpRecv, MpTransport};

use crate::wire::{MpDatagram, WireFrameKind};

fn to_core_kind(kind: WireFrameKind) -> MpFrameKind {
    match kind {
        WireFrameKind::Packet => MpFrameKind::Packet,
        WireFrameKind::Cmd => MpFrameKind::Cmd,
        WireFrameKind::Reply => MpFrameKind::Reply,
        WireFrameKind::Ack => MpFrameKind::Ack,
    }
}

/// Shared knowledge of where MP datagrams should be sent. The host relays
/// to every known peer; a client only ever sends to the host. Updated by
/// the room's control-channel logic as players join/leave
/// (`docs/design/design_lan.md` §5.6).
#[derive(Default)]
pub struct PeerTable {
    inner: Mutex<Vec<(u8, SocketAddr)>>,
}

impl PeerTable {
    pub fn set(&self, peers: Vec<(u8, SocketAddr)>) {
        *self.inner.lock().unwrap_or_else(|e| e.into_inner()) = peers;
    }

    fn snapshot(&self) -> Vec<(u8, SocketAddr)> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Shared, mutable adaptive-pacing output. The pacing controller
/// (`crate::pacing::Controller`) lives on the room-control thread and
/// publishes here; [`NetTransport::link_hints`] just reads the latest
/// snapshot with no locking cost beyond the mutex itself.
#[derive(Default)]
pub struct SharedHints {
    inner: Mutex<LinkHints>,
}

impl SharedHints {
    pub fn set(&self, hints: LinkHints) {
        *self.inner.lock().unwrap_or_else(|e| e.into_inner()) = hints;
    }

    pub fn get(&self) -> LinkHints {
        *self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Inbound {
    kind: WireFrameKind,
    sender_id: u8,
    aid: u16,
    timestamp_us: u64,
    runahead_us: u32,
    payload: Vec<u8>,
}

/// UDP-backed [`MpTransport`]. Construct via [`NetTransport::new`], which
/// binds the socket and spawns the RX pump thread; both are torn down when
/// this value (and its `Arc<AtomicBool>` shutdown flag) drop.
pub struct NetTransport {
    socket: Arc<UdpSocket>,
    self_id: u8,
    host_id: u8,
    peers: Arc<PeerTable>,
    hints: Arc<SharedHints>,
    regular_rx: Receiver<Inbound>,
    reply_rx: Receiver<Inbound>,
    host_gone: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    send_seq: AtomicU32,
}

impl NetTransport {
    /// Wraps an already-bound UDP socket and starts relaying. Taking an
    /// already-bound socket (rather than binding internally) lets the
    /// guest path bind early -- to learn its local port for the `Hello`
    /// message -- and only construct the `NetTransport` once `self_id` is
    /// known from the host's `Welcome` reply.
    ///
    /// # Errors
    /// Returns any error from configuring the socket (setting its read
    /// timeout).
    pub fn from_socket(
        socket: UdpSocket,
        self_id: u8,
        host_id: u8,
        peers: Arc<PeerTable>,
        hints: Arc<SharedHints>,
    ) -> std::io::Result<Self> {
        let socket = Arc::new(socket);
        // A read timeout lets the RX thread periodically re-check the
        // shutdown flag instead of blocking forever in `recv_from`.
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;

        let (regular_tx, regular_rx) = std::sync::mpsc::channel();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        let host_gone = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));

        spawn_rx_pump(
            Arc::clone(&socket),
            host_id,
            regular_tx,
            reply_tx,
            Arc::clone(&host_gone),
            Arc::clone(&shutdown),
        );

        Ok(NetTransport {
            socket,
            self_id,
            host_id,
            peers,
            hints,
            regular_rx,
            reply_rx,
            host_gone,
            shutdown,
            send_seq: AtomicU32::new(0),
        })
    }

    /// Local address the socket is bound to (for advertising in `Hello`'s
    /// `udp_port` field).
    ///
    /// # Errors
    /// Propagates any error from the underlying `local_addr` call.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    fn next_seq(&self) -> u32 {
        self.send_seq.fetch_add(1, Ordering::Relaxed)
    }

    fn send_to_peers(
        &self,
        kind: WireFrameKind,
        data: &[u8],
        timestamp_us: u64,
        aid: u16,
        runahead_us: u32,
    ) -> usize {
        let dgram = MpDatagram {
            sender_id: self.self_id,
            kind,
            aid,
            send_seq: self.next_seq(),
            timestamp_us,
            runahead_us,
            payload: data.to_vec(),
        };
        let bytes = dgram.encode();

        let targets: Vec<SocketAddr> = if self.self_id == self.host_id {
            // Host: broadcast to every known peer.
            self.peers.snapshot().into_iter().map(|(_, addr)| addr).collect()
        } else {
            // Client: unicast to the host's address.
            self.peers
                .snapshot()
                .into_iter()
                .find(|&(id, _)| id == self.host_id)
                .map(|(_, addr)| addr)
                .into_iter()
                .collect()
        };

        for addr in &targets {
            let _ = self.socket.send_to(&bytes, addr);
        }
        data.len()
    }
}

impl MpTransport for NetTransport {
    fn begin(&mut self) {}
    fn end(&mut self) {}

    fn send_packet(&mut self, data: &[u8], timestamp_us: u64) -> usize {
        self.send_to_peers(WireFrameKind::Packet, data, timestamp_us, 0, 0)
    }

    fn send_cmd(&mut self, data: &[u8], timestamp_us: u64) -> usize {
        self.send_to_peers(WireFrameKind::Cmd, data, timestamp_us, 0, 0)
    }

    fn send_reply(&mut self, data: &[u8], timestamp_us: u64, aid: u16) -> usize {
        self.send_to_peers(WireFrameKind::Reply, data, timestamp_us, aid, 0)
    }

    fn send_ack(&mut self, data: &[u8], timestamp_us: u64, runahead_us: u32) -> usize {
        self.send_to_peers(WireFrameKind::Ack, data, timestamp_us, 0, runahead_us)
    }

    fn recv_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        match self.regular_rx.try_recv() {
            Ok(frame) => deliver(buf, frame),
            Err(TryRecvError::Empty) => {
                if self.host_gone.load(Ordering::Relaxed) {
                    MpRecv::HostGone
                } else {
                    MpRecv::None
                }
            }
            Err(TryRecvError::Disconnected) => MpRecv::HostGone,
        }
    }

    fn recv_host_packet(&mut self, buf: &mut [u8], timeout: Duration) -> MpRecv {
        match self.regular_rx.recv_timeout(timeout) {
            Ok(frame) => deliver(buf, frame),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if self.host_gone.load(Ordering::Relaxed) { MpRecv::HostGone } else { MpRecv::None }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => MpRecv::HostGone,
        }
    }

    fn recv_replies(&mut self, buf: &mut [u8], timestamp_us: u64, aid_mask: u16) -> u16 {
        let mut answered = 0u16;
        let mut offset = 0usize;
        // Bounded wall-clock budget: this is called synchronously from the
        // TX-complete path, so it must not stall the caller indefinitely
        // even if a client never replies.
        let deadline = std::time::Instant::now() + Duration::from_millis(50);
        loop {
            if answered & aid_mask == aid_mask || std::time::Instant::now() >= deadline {
                break;
            }
            match self.reply_rx.recv_timeout(Duration::from_millis(5)) {
                Ok(frame) => {
                    if frame.aid >= 16 || aid_mask & (1 << frame.aid) == 0 {
                        continue;
                    }
                    if frame.timestamp_us.abs_diff(timestamp_us) > 32_000 {
                        continue;
                    }
                    let end = (offset + frame.payload.len()).min(buf.len());
                    if end > offset {
                        buf[offset..end].copy_from_slice(&frame.payload[..end - offset]);
                        offset = end;
                    }
                    answered |= 1 << frame.aid;
                }
                Err(_) => break,
            }
        }
        answered
    }

    fn link_hints(&self) -> LinkHints {
        self.hints.get()
    }
}

impl Drop for NetTransport {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

fn deliver(buf: &mut [u8], frame: Inbound) -> MpRecv {
    let len = frame.payload.len().min(buf.len());
    buf[..len].copy_from_slice(&frame.payload[..len]);
    MpRecv::Frame {
        len,
        kind: to_core_kind(frame.kind),
        timestamp_us: frame.timestamp_us,
        runahead_us: frame.runahead_us,
    }
}

fn spawn_rx_pump(
    socket: Arc<UdpSocket>,
    host_id: u8,
    regular_tx: Sender<Inbound>,
    reply_tx: Sender<Inbound>,
    host_gone: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 4096];
        loop {
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            match socket.recv_from(&mut buf) {
                Ok((len, _addr)) => {
                    let Ok(dgram) = MpDatagram::decode(&buf[..len]) else { continue };
                    let inbound = Inbound {
                        kind: dgram.kind,
                        sender_id: dgram.sender_id,
                        aid: dgram.aid,
                        timestamp_us: dgram.timestamp_us,
                        runahead_us: dgram.runahead_us,
                        payload: dgram.payload,
                    };
                    let _ = inbound.sender_id; // Currently informational only; host-only filtering happens via `PeerTable`/room logic upstream.
                    match dgram.kind {
                        WireFrameKind::Reply => {
                            let _ = reply_tx.send(inbound);
                        }
                        _ => {
                            let _ = regular_tx.send(inbound);
                        }
                    }
                    let _ = host_id;
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(_) => {
                    host_gone.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
    });
}
