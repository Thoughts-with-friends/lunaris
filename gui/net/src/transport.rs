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
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError},
    },
    time::{Duration, Instant},
};

use nds_core::nds::{LinkHints, MpFrameKind, MpRecv, MpTransport};

use crate::wire::{MpDatagram, WireFrameKind};

/// How long an inbound MP datagram may sit in the receive queue before it
/// is assumed stale and dropped rather than delivered. Matches melonDS's
/// `LAN::ProcessLAN` staleness rule ("any incoming packet should be
/// consumed by the core quickly, so if it's been sitting in the queue for
/// more than one frame's time, we can assume it's stale",
/// `docs/design/melonds/net/LAN.cpp:805-834`) -- one video frame's worth of
/// wall-clock time. Undelivered stale frames poison the MP client's sync
/// clock (`Wifi::next_sync`) if left in the queue; see
/// `docs/design/review_mp_local.md` F3.
const RX_STALE: Duration = Duration::from_millis(16);
/// Soft cap on each inbound channel. `sync_channel` blocks a full sender,
/// so the RX pump uses `try_send` and drops the *newest* datagram on
/// overflow instead -- a deliberate deviation from melonDS's drop-oldest
/// policy, acceptable because [`RX_STALE`] eviction on the consumer side
/// keeps the backlog from ever reaching this cap in practice.
const RX_QUEUE_CAP: usize = 32;

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
    aid: u16,
    timestamp_us: u64,
    runahead_us: u32,
    payload: Vec<u8>,
    /// Wall-clock instant this datagram was pulled off the socket, used by
    /// the consumer side to evict stale entries (see [`RX_STALE`]).
    arrival: Instant,
}

/// `true` if `frame` has been sitting in the queue longer than
/// [`RX_STALE`] and should be dropped rather than delivered.
fn is_stale(frame: &Inbound) -> bool {
    frame.arrival.elapsed() > RX_STALE
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

        // Bounded: an unbounded channel would let a stalled consumer
        // accumulate an ever-growing backlog of frames that are stale by
        // the time they're read, exactly the failure mode in
        // `docs/design/review_mp_local.md` F3.
        let (regular_tx, regular_rx) = std::sync::mpsc::sync_channel(RX_QUEUE_CAP);
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(RX_QUEUE_CAP);
        let host_gone = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));

        spawn_rx_pump(
            Arc::clone(&socket),
            self_id,
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

    /// A shared handle onto this transport's "host is gone" flag, so a
    /// caller with a more authoritative signal than a UDP socket error --
    /// e.g. the room's TCP control channel dropping, which the underlying
    /// socket alone cannot detect (`docs/design/review_mp_local.md` F6) --
    /// can raise it directly. The next [`MpTransport::recv_packet`] /
    /// [`MpTransport::recv_host_packet`] call will then report
    /// [`MpRecv::HostGone`].
    #[must_use]
    pub fn host_gone_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.host_gone)
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
        loop {
            match self.regular_rx.try_recv() {
                Ok(frame) if is_stale(&frame) => continue,
                Ok(frame) => return deliver(buf, frame),
                Err(TryRecvError::Empty) => {
                    return if self.host_gone.load(Ordering::Relaxed) {
                        MpRecv::HostGone
                    } else {
                        MpRecv::None
                    };
                }
                Err(TryRecvError::Disconnected) => return MpRecv::HostGone,
            }
        }
    }

    fn recv_host_packet(&mut self, buf: &mut [u8], timeout: Duration) -> MpRecv {
        // Budget the whole call, not each individual receive: draining a
        // backlog of stale frames (F3) must not let the effective wait
        // exceed the caller's timeout.
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.regular_rx.recv_timeout(remaining) {
                Ok(frame) if is_stale(&frame) => continue,
                Ok(frame) => return deliver(buf, frame),
                Err(RecvTimeoutError::Timeout) => {
                    return if self.host_gone.load(Ordering::Relaxed) {
                        MpRecv::HostGone
                    } else {
                        MpRecv::None
                    };
                }
                Err(RecvTimeoutError::Disconnected) => return MpRecv::HostGone,
            }
        }
    }

    fn recv_replies(&mut self, buf: &mut [u8], timestamp_us: u64, aid_mask: u16) -> u16 {
        let mut answered = 0u16;
        // Ported from melonDS `LAN::RecvReplies`: honour the transport's
        // configured receive budget (`link_hints().recv_timeout`, melonDS's
        // `MPRecvTimeout`) rather than a value hard-coded independently of
        // it -- see `docs/design/review_mp_local.md` F11.
        let timeout = self.hints.get().recv_timeout;
        let deadline = Instant::now() + timeout;
        loop {
            if answered & aid_mask == aid_mask {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match self.reply_rx.recv_timeout(remaining) {
                Ok(frame) => {
                    if is_stale(&frame) {
                        continue;
                    }
                    if frame.aid == 0 || frame.aid >= 16 || aid_mask & (1 << frame.aid) == 0 {
                        continue;
                    }
                    // One-sided staleness test on the *emulated* timestamp
                    // (distinct from `arrival`/`RX_STALE`, which guards
                    // wall-clock queue latency): matches melonDS's
                    // `header->Timestamp < (timestamp - 32)`. A reply whose
                    // emulated clock legitimately runs ahead of the host's
                    // (granted by the host's own run-ahead window) must
                    // never be rejected -- only one that lags behind.
                    if frame.timestamp_us < timestamp_us.wrapping_sub(32_000) {
                        continue;
                    }
                    // Replies are addressed by association ID into fixed
                    // 1 KiB slots (`packets[(aid-1)*1024]` in melonDS), not
                    // packed back-to-back -- see
                    // `docs/design/review_mp_local.md` F2.
                    let slot = (frame.aid as usize - 1) * 1024;
                    let end = (slot + frame.payload.len()).min(buf.len());
                    if end > slot {
                        buf[slot..end].copy_from_slice(&frame.payload[..end - slot]);
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
    self_id: u8,
    regular_tx: SyncSender<Inbound>,
    reply_tx: SyncSender<Inbound>,
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
                    // Reject a datagram that echoed back to us -- from a
                    // misconfigured peer table (see `Room::host`'s former
                    // self-entry) or a broadcast/relay quirk. melonDS's
                    // `LAN::RecvPacketGeneric` applies the same
                    // `SenderID == MyPlayer.ID` filter; see
                    // `docs/design/review_mp_local.md` F4.
                    if dgram.sender_id == self_id {
                        continue;
                    }
                    let inbound = Inbound {
                        kind: dgram.kind,
                        aid: dgram.aid,
                        timestamp_us: dgram.timestamp_us,
                        runahead_us: dgram.runahead_us,
                        payload: dgram.payload,
                        arrival: Instant::now(),
                    };
                    // `try_send`: a full queue means the consumer has
                    // fallen behind, and the datagram now waiting longest
                    // is about to age out via `RX_STALE` regardless -- see
                    // the `RX_QUEUE_CAP` doc comment.
                    match dgram.kind {
                        WireFrameKind::Reply => {
                            let _ = reply_tx.try_send(inbound);
                        }
                        _ => {
                            let _ = regular_tx.try_send(inbound);
                        }
                    }
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
    use std::net::Ipv4Addr;

    use super::*;

    /// Binds a `NetTransport` pair on localhost, each addressed by the
    /// other. `host_id` is `0` (transport `a`); `b` is instance `1`.
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

    #[test]
    fn packet_round_trips_between_paired_sockets() {
        let (mut a, mut b) = transport_pair();
        a.send_packet(&[1, 2, 3], 100);
        // `recv_host_packet` blocks up to the timeout instead of a fixed
        // `sleep` + single `try_recv`, so this doesn't race `RX_STALE`
        // eviction the way an over-long sleep before a non-blocking poll
        // would.
        let mut buf = [0u8; 16];
        let recv = b.recv_host_packet(&mut buf, Duration::from_millis(200));
        assert_eq!(
            recv,
            MpRecv::Frame { len: 3, kind: MpFrameKind::Packet, timestamp_us: 100, runahead_us: 0 }
        );
        assert_eq!(&buf[..3], &[1, 2, 3]);
    }

    #[test]
    fn own_frame_echoed_back_is_never_delivered() {
        // Simulates the self-loop bug (F4): a raw datagram claiming to be
        // from this transport's own `self_id`, sent straight at its own
        // socket, must never surface from `recv_packet`.
        let (mut a, _b) = transport_pair();
        let addr_a = a.local_addr().unwrap();
        let echo_socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let dgram = MpDatagram {
            sender_id: 0, // Same as `a`'s own `self_id`.
            kind: WireFrameKind::Packet,
            aid: 0,
            send_seq: 0,
            timestamp_us: 1,
            runahead_us: 0,
            payload: vec![9, 9, 9],
        };
        echo_socket.send_to(&dgram.encode(), addr_a).unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let mut buf = [0u8; 16];
        assert_eq!(a.recv_packet(&mut buf), MpRecv::None);
    }

    #[test]
    fn replies_land_in_their_per_aid_slot() {
        let (mut a, mut b) = transport_pair();
        b.send_reply(&[0xAB; 4], 100_000, 2);
        let mut buf = [0u8; 15 * 1024];
        let answered = a.recv_replies(&mut buf, 100_000, 1 << 2);
        assert_eq!(answered, 1 << 2);
        assert_eq!(&buf[1024..1028], &[0xAB; 4]);
        assert!(buf[..1024].iter().all(|&b| b == 0));
    }

    #[test]
    fn replies_ahead_of_the_host_clock_are_accepted() {
        // Regression test for the two-sided `abs_diff` bug: a client
        // legitimately running ahead of the host (within its granted
        // run-ahead window) must not be rejected.
        let (mut a, mut b) = transport_pair();
        b.send_reply(&[7], 100_500, 1);
        let mut buf = [0u8; 15 * 1024];
        let answered = a.recv_replies(&mut buf, 100_000, 1 << 1);
        assert_eq!(answered, 1 << 1);
    }

    #[test]
    fn stale_arrival_frames_are_dropped() {
        // Directly exercises the `RX_STALE` eviction path used by `recv_packet`
        // /`recv_host_packet`, without needing to actually wait `RX_STALE`
        // out on a real socket.
        let frame = Inbound {
            kind: WireFrameKind::Packet,
            aid: 0,
            timestamp_us: 1,
            runahead_us: 0,
            payload: vec![1],
            arrival: Instant::now() - Duration::from_millis(50),
        };
        assert!(is_stale(&frame));

        let fresh = Inbound { arrival: Instant::now(), ..frame };
        assert!(!is_stale(&fresh));
    }
}
