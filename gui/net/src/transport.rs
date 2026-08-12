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
    time::{Duration, Instant},
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
    /// Room-level player id of the sender. Needed by
    /// [`NetTransport::recv_replies`] to know when *every* connected peer has
    /// been heard from -- including one whose reply was a zero-length
    /// keep-alive carrying no AID. See
    /// `docs/design/local-mp-melonds-parity-2.md` F5.
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

        // melonDS's `RecvReplies` releases on *either* of two conditions
        // (`docs/design/melonds/net/LocalMP.cpp:295-360`): every addressed AID
        // sent data, or every connected instance has been heard from at all.
        // The second is what makes the zero-length keep-alive reply
        // (`Wifi.cpp:1496-1503`, mirrored in `Wifi::step_rx`) do its job --
        // without it the host burns its whole receive budget, every CMD round,
        // waiting for a reply that already arrived carrying no payload. See
        // `docs/design/local-mp-melonds-parity-2.md` F5.
        //
        // Room ids are bounded to `0..16`; anything outside that range
        // contributes no bit rather than overflowing the shift.
        let bit = |id: u8| -> u16 { if id < 16 { 1u16 << id } else { 0 } };
        let connected: u16 =
            self.peers.snapshot().iter().fold(bit(self.self_id), |acc, &(id, _)| acc | bit(id));
        let mut heard = bit(self.self_id);

        // Unlike melonDS's `MPStatus.ConnectedBitmask`, this peer table is
        // published asynchronously by the room's control thread, so an empty
        // one means "topology not known yet" at least as often as it means
        // "everybody left". Only trust the sender-based release when the table
        // actually names a peer; otherwise fall back to waiting out the
        // receive budget, which is the pre-existing behaviour.
        let topology_known = connected != bit(self.self_id);

        // Honour the transport's configured receive budget
        // (`link_hints().recv_timeout`, melonDS's `MPRecvTimeout`) rather than
        // a value hard-coded independently of it.
        let timeout = self.hints.get().recv_timeout;
        let deadline = Instant::now() + timeout;
        loop {
            if answered & aid_mask == aid_mask || (topology_known && heard == connected) {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Ok(frame) = self.reply_rx.recv_timeout(remaining) else { break };

            // Count the sender regardless of whether its reply carried data:
            // melonDS's `myinstmask |= (1 << pktheader.SenderID)`.
            heard |= bit(frame.sender_id);

            // A zero-length reply names no AID and sets no `answered` bit --
            // its only job was the `heard` bookkeeping above.
            if frame.payload.is_empty() || frame.aid == 0 || frame.aid >= 16 {
                continue;
            }
            if aid_mask & (1 << frame.aid) == 0 {
                continue;
            }
            // One-sided staleness test on the emulated clock, following
            // melonDS's `header->Timestamp < (timestamp - 32)`: a reply whose
            // clock legitimately runs *ahead* of the host's (granted by the
            // host's own run-ahead window) must never be rejected -- only one
            // that lags behind. Replaces a two-sided `abs_diff` test, which
            // rejected exactly the run-ahead case the ack frame authorises.
            //
            // Saturating rather than wrapping, deliberately diverging from
            // melonDS: its tolerance is 32µs, so `timestamp - 32` underflows
            // only in the first 32µs of a session. lunaris's tolerance is
            // 32ms, and wrapping there makes *every* reply vacuously stale for
            // the first 32ms -- long enough to cover a real handshake.
            if frame.timestamp_us + 32_000 < timestamp_us {
                continue;
            }
            // Replies are addressed by association ID into fixed 1 KiB slots,
            // exactly as `Wifi::mp_client_reply_rx` reads them back
            // (`mp_client_replies[(client - 1) * 1024]`, melonDS's
            // `packets[(aid-1)*1024]`). Packing them back-to-back from offset
            // zero -- as this used to -- put every reply somewhere the core
            // never looks.
            let slot = (frame.aid as usize - 1) * 1024;
            let end = (slot + frame.payload.len()).min(buf.len());
            if end > slot {
                buf[slot..end].copy_from_slice(&frame.payload[..end - slot]);
            }
            answered |= 1 << frame.aid;
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

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Instant};

    use super::*;

    /// Binds a `NetTransport` pair on localhost, each addressed by the other.
    /// `host_id` is `0` (transport `a`); `b` is instance `1`.
    fn transport_pair() -> (NetTransport, NetTransport) {
        let sock_a = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let sock_b = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr_a = sock_a.local_addr().unwrap();
        let addr_b = sock_b.local_addr().unwrap();

        let peers_a = Arc::new(PeerTable::default());
        peers_a.set(vec![(1, addr_b)]);
        let peers_b = Arc::new(PeerTable::default());
        peers_b.set(vec![(0, addr_a)]);

        let hints_a = Arc::new(SharedHints::default());
        hints_a.set(LinkHints { runahead_us: 1000, recv_timeout: Duration::from_millis(200) });
        let hints_b = Arc::new(SharedHints::default());
        hints_b.set(LinkHints { runahead_us: 1000, recv_timeout: Duration::from_millis(200) });

        let a = NetTransport::from_socket(sock_a, 0, 0, peers_a, hints_a).unwrap();
        let b = NetTransport::from_socket(sock_b, 1, 0, peers_b, hints_b).unwrap();
        (a, b)
    }

    /// Replies must land in the fixed per-AID 1 KiB slot the core reads them
    /// back from (`Wifi::mp_client_reply_rx` indexes
    /// `mp_client_replies[(client - 1) * 1024]`). Packing them back-to-back
    /// from offset zero put every reply somewhere the core never looks.
    #[test]
    fn replies_land_in_their_per_aid_slot() {
        let (mut a, mut b) = transport_pair();
        b.send_reply(&[0xAB; 4], 100_000, 2);

        let mut buf = [0u8; 15 * 1024];
        let answered = a.recv_replies(&mut buf, 100_000, 1 << 2);

        assert_eq!(answered, 1 << 2);
        assert_eq!(&buf[1024..1028], &[0xAB; 4], "AID 2 occupies the second 1 KiB slot");
        assert!(buf[..1024].iter().all(|&b| b == 0), "AID 1's slot must be untouched");
    }

    /// A zero-length keep-alive reply (aid 0) carries no data and must not set
    /// an `answered` bit -- but it *must* still release the host's wait,
    /// because it proves the peer is alive and had nothing to send. Without
    /// this the host burns its full receive budget on every CMD round,
    /// blocking the emulator thread inside a microsecond-scale hardware phase.
    /// `docs/design/local-mp-melonds-parity-2.md` F5.
    #[test]
    fn blank_reply_releases_the_wait_without_marking_an_aid() {
        let (mut a, mut b) = transport_pair();
        b.send_reply(&[], 100_000, 0);

        let mut buf = [0u8; 15 * 1024];
        let start = Instant::now();
        let answered = a.recv_replies(&mut buf, 100_000, 1 << 1);

        assert_eq!(answered, 0, "a blank reply names no AID");
        assert!(
            start.elapsed() < Duration::from_millis(150),
            "the wait must end as soon as every connected peer has been heard from, not after              the full 200ms receive budget (took {:?})",
            start.elapsed()
        );
    }
}
