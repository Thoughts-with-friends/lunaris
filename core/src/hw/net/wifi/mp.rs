//! Multiplayer (MP) transport abstraction.
//!
//! [`Wifi`](super::Wifi) never touches a socket directly: every frame it
//! sends or receives passes through an [`MpTransport`] implementation
//! supplied by the frontend. This keeps `nds-core` free of any networking
//! dependency, and makes headless testing (see
//! `core/examples/mp_loopback.rs`) possible by swapping in
//! [`LoopbackTransport`] instead of a real socket-backed implementation.
//!
//! See `docs/design/design_lan.md` §8.1.

use std::{
    sync::mpsc::{Receiver, Sender, TryRecvError},
    time::Duration,
};

/// Which of the four MP frame categories a received frame belongs to. The
/// wire protocol tags this explicitly (`docs/design/design_lan.md` §5.4's
/// `mp_type` field) rather than making the receiver sniff the 802.11 frame
/// body for it — command/reply/ack frames are a lunaris-level relay concept,
/// not something derivable from arbitrary frame content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpFrameKind {
    /// Regular data/beacon/auth/association/deauth traffic.
    Packet,
    /// Host multiplayer command frame.
    Cmd,
    /// Client multiplayer reply frame.
    Reply,
    /// Host multiplayer acknowledgement frame.
    Ack,
}

/// Result of a receive attempt on an [`MpTransport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MpRecv {
    /// No frame was available (or the timeout elapsed).
    None,
    /// A frame was received.
    Frame {
        /// Number of bytes written into the caller's buffer.
        len: usize,
        /// Which MP frame category this is.
        kind: MpFrameKind,
        /// Sender's emulated microsecond clock at the time of transmission.
        timestamp_us: u64,
        /// Host-granted run-ahead window in microseconds (ack frames only;
        /// zero otherwise). See `docs/design/design_lan.md` §9.
        runahead_us: u32,
    },
    /// The host is no longer reachable; the MP session must end.
    HostGone,
}

/// Snapshot of the adaptive link parameters currently in effect.
///
/// See `docs/design/design_lan.md` §9.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkHints {
    /// How far (in microseconds) a client may run ahead of the host before
    /// its next mandatory sync point.
    pub runahead_us: u32,
    /// How long [`MpTransport::recv_host_packet`] may block per call.
    pub recv_timeout: Duration,
}

impl Default for LinkHints {
    fn default() -> Self {
        // Deliberately **below** melonDS's 25ms `MPInterface::RecvTimeout`
        // default, and this is the one figure here that must not be raised to
        // match it.
        //
        // A client past its sync point re-enters `recv_host_packet` on every
        // 8µs tick, so this timeout is what one tick of *emulated* time costs
        // in *wall* time whenever the host is not answering. melonDS can
        // afford 25ms because its host is answering. When it is not -- a host
        // that never opens a command round, which is the failure this port is
        // still chasing -- the client pays 25ms of wall time per 8µs of
        // emulated time and grinds to a crawl, which the game reads as the
        // link dying. Raising this from 8ms to 25ms was measured doing exactly
        // that: the guest's radio clock advanced at roughly a third of the
        // host's.
        //
        // What the lower value costs is a client giving up on a frame still in
        // flight and retrying on the next tick, which is cheap.
        LinkHints { runahead_us: 1000, recv_timeout: Duration::from_millis(8) }
    }
}

/// Frontend-supplied transport for DS multiplayer frames.
///
/// All methods take `&mut self` so a socket-backed implementation can own
/// its I/O state without interior mutability. Implementations must be
/// [`Send`] because the emulator core runs on whichever thread owns
/// [`crate::nds::NDS`], which frontends are free to choose.
///
/// # Errors
/// This trait itself is infallible by design: transient I/O failures are
/// reported as [`MpRecv::None`] (nothing available) or [`MpRecv::HostGone`]
/// (session must end) rather than via `Result`, since [`Wifi`](super::Wifi)
/// has no error-recovery path of its own — it just keeps polling.
pub trait MpTransport: Send {
    /// Called when Wi-Fi hardware power turns on (`POWCNT2` bit 1 and
    /// `W_PowerUS` bit 0 both permit it).
    fn begin(&mut self);

    /// Called when Wi-Fi hardware power turns off.
    fn end(&mut self);

    /// Broadcasts a regular MP data/beacon/auth/assoc frame to all
    /// MP-ready peers. Returns the number of bytes accepted.
    fn send_packet(&mut self, data: &[u8], timestamp_us: u64) -> usize;

    /// Broadcasts a host MP command frame (TX slot 1). Returns the number
    /// of bytes accepted.
    fn send_cmd(&mut self, data: &[u8], timestamp_us: u64) -> usize;

    /// Unicasts a client MP reply frame to the host. Returns the number of
    /// bytes accepted.
    fn send_reply(&mut self, data: &[u8], timestamp_us: u64, aid: u16) -> usize;

    /// Broadcasts a host MP acknowledgement frame, carrying the run-ahead
    /// window granted to clients. Returns the number of bytes accepted.
    fn send_ack(&mut self, data: &[u8], timestamp_us: u64, runahead_us: u32) -> usize;

    /// Non-blocking poll for any inbound frame (regular RX path).
    fn recv_packet(&mut self, buf: &mut [u8]) -> MpRecv;

    /// Bounded blocking wait for the next frame from the host. Used by MP
    /// clients at their sync point (`docs/design/design_lan.md` §8.3); the
    /// timeout must never exceed [`LinkHints::recv_timeout`] by more than a
    /// small scheduling margin.
    fn recv_host_packet(&mut self, buf: &mut [u8], timeout: Duration) -> MpRecv;

    /// Host-only: collects reply frames from the clients named in `aid_mask`.
    /// Returns the bitmask of AIDs that replied.
    ///
    /// # Contract
    /// Every implementation must obey all of the following. There are three
    /// implementations in this workspace and they diverged once already, which
    /// is what let the headless harness report success while real play failed
    /// (`docs/design/review_mp_local2.md` §5, P1-2):
    ///
    /// * **Slot addressing.** A reply from association ID `aid` is written at
    ///   byte offset `(aid - 1) * 1024` in `buf`, at most 1024 bytes. Never at
    ///   a running offset. This is the contract with
    ///   [`Wifi::mp_client_reply_rx`](super::Wifi::mp_client_reply_rx), which
    ///   reads `mp_client_replies[(aid - 1) * 1024 .. +1024]` — melonDS's
    ///   `MPClientReplies[15][1024]`.
    /// * **AID validity.** `aid == 0` (which would underflow the slot index)
    ///   and `aid >= 16` are rejected without writing anything.
    /// * **Mask filter.** A reply whose AID is not set in `aid_mask` is ignored.
    /// * **Staleness.** One-sided: reject only a reply whose timestamp *lags*
    ///   `timestamp_us` by more than the tolerance. A reply running ahead is
    ///   valid — the host's own ack frame grants clients that run-ahead window.
    ///   Use saturating arithmetic so nothing is vacuously stale at session
    ///   start.
    /// * **Blank keep-alive.** A zero-length reply names no AID and sets no
    ///   bit in the result, but still counts as having heard from its sender.
    /// * **Release condition.** Return as soon as *either* every AID in
    ///   `aid_mask` has answered *or* every connected instance has been heard
    ///   from; otherwise wait out [`LinkHints::recv_timeout`].
    fn recv_replies(&mut self, buf: &mut [u8], timestamp_us: u64, aid_mask: u16) -> u16;

    /// Current adaptive link parameters (see
    /// `docs/design/design_lan.md` §9). Polled once per MP sync point.
    fn link_hints(&self) -> LinkHints;
}

/// In-process [`MpTransport`] pair connected by channels, with no socket
/// involved. Used by the headless verification harness
/// (`core/examples/mp_loopback.rs`) and by unit tests, so Wi-Fi/MP logic can
/// be exercised without any real networking.
///
/// Frames placed on the wire are tagged with the sending peer's id so the
/// receiver can apply the same host/guest filtering rules a real transport
/// would (`docs/design/design_lan.md` §5.6).
///
/// Reply frames travel on a channel separate from every other frame kind,
/// mirroring [`super::super::wifi`]'s `NetTransport` (`gui/net`'s socket
/// transport also keeps a `regular_rx`/`reply_rx` split for the same
/// reason). Without this, [`LoopbackTransport::recv_packet`]/
/// [`LoopbackTransport::recv_host_packet`] (driven every regular tick by
/// `Wifi::check_rx`) can steal a reply meant for
/// [`LoopbackTransport::recv_replies`] (driven once per CMD round by
/// `Wifi::tx_phase_transmit_done`) clean off the wire, permanently losing
/// it -- `recv_replies` never retries.
pub struct LoopbackTransport {
    peer_id: u8,
    host_id: u8,
    tx: Sender<LoopbackFrame>,
    rx: Receiver<LoopbackFrame>,
    reply_tx: Sender<LoopbackFrame>,
    reply_rx: Receiver<LoopbackFrame>,
    hints: LinkHints,
}

struct LoopbackFrame {
    sender_id: u8,
    kind: MpFrameKind,
    data: Vec<u8>,
    timestamp_us: u64,
    aid: u16,
    runahead_us: u32,
}

impl LoopbackTransport {
    /// Builds a connected pair: `(host, client)`. The host is always peer id
    /// 0; the client is peer id 1.
    pub fn new_pair() -> (LoopbackTransport, LoopbackTransport) {
        let (tx_a, rx_a) = std::sync::mpsc::channel();
        let (tx_b, rx_b) = std::sync::mpsc::channel();
        let (reply_tx_a, reply_rx_a) = std::sync::mpsc::channel();
        let (reply_tx_b, reply_rx_b) = std::sync::mpsc::channel();
        let host = LoopbackTransport {
            peer_id: 0,
            host_id: 0,
            tx: tx_b,
            rx: rx_a,
            reply_tx: reply_tx_b,
            reply_rx: reply_rx_a,
            hints: LinkHints::default(),
        };
        let client = LoopbackTransport {
            peer_id: 1,
            host_id: 0,
            tx: tx_a,
            rx: rx_b,
            reply_tx: reply_tx_a,
            reply_rx: reply_rx_b,
            hints: LinkHints::default(),
        };
        (host, client)
    }

    /// Overrides the simulated link hints, letting tests exercise the
    /// adaptive-pacing consumer without a real controller.
    pub fn set_hints(&mut self, hints: LinkHints) {
        self.hints = hints;
    }

    fn poll(&mut self, buf: &mut [u8], filter_host_only: bool) -> MpRecv {
        loop {
            match self.rx.try_recv() {
                Ok(frame) => {
                    if filter_host_only && frame.sender_id != self.host_id {
                        continue;
                    }
                    let len = frame.data.len().min(buf.len());
                    buf[..len].copy_from_slice(&frame.data[..len]);
                    return MpRecv::Frame {
                        len,
                        kind: frame.kind,
                        timestamp_us: frame.timestamp_us,
                        runahead_us: frame.runahead_us,
                    };
                }
                Err(TryRecvError::Empty) => return MpRecv::None,
                Err(TryRecvError::Disconnected) => return MpRecv::HostGone,
            }
        }
    }
}

impl MpTransport for LoopbackTransport {
    fn begin(&mut self) {}
    fn end(&mut self) {}

    fn send_packet(&mut self, data: &[u8], timestamp_us: u64) -> usize {
        let frame = LoopbackFrame {
            sender_id: self.peer_id,
            kind: MpFrameKind::Packet,
            data: data.to_vec(),
            timestamp_us,
            aid: 0,
            runahead_us: 0,
        };
        let len = data.len();
        let _ = self.tx.send(frame);
        len
    }

    fn send_cmd(&mut self, data: &[u8], timestamp_us: u64) -> usize {
        let frame = LoopbackFrame {
            sender_id: self.peer_id,
            kind: MpFrameKind::Cmd,
            data: data.to_vec(),
            timestamp_us,
            aid: 0,
            runahead_us: 0,
        };
        let len = data.len();
        let _ = self.tx.send(frame);
        len
    }

    fn send_reply(&mut self, data: &[u8], timestamp_us: u64, aid: u16) -> usize {
        let frame = LoopbackFrame {
            sender_id: self.peer_id,
            kind: MpFrameKind::Reply,
            data: data.to_vec(),
            timestamp_us,
            aid,
            runahead_us: 0,
        };
        let len = data.len();
        let _ = self.reply_tx.send(frame);
        len
    }

    fn send_ack(&mut self, data: &[u8], timestamp_us: u64, runahead_us: u32) -> usize {
        let frame = LoopbackFrame {
            sender_id: self.peer_id,
            kind: MpFrameKind::Ack,
            data: data.to_vec(),
            timestamp_us,
            aid: 0,
            runahead_us,
        };
        let len = data.len();
        let _ = self.tx.send(frame);
        len
    }

    fn recv_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        self.poll(buf, false)
    }

    fn recv_host_packet(&mut self, buf: &mut [u8], timeout: Duration) -> MpRecv {
        match self.rx.recv_timeout(timeout) {
            Ok(frame) => {
                if frame.sender_id != self.host_id {
                    return MpRecv::None;
                }
                let len = frame.data.len().min(buf.len());
                buf[..len].copy_from_slice(&frame.data[..len]);
                MpRecv::Frame {
                    len,
                    kind: frame.kind,
                    timestamp_us: frame.timestamp_us,
                    runahead_us: frame.runahead_us,
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => MpRecv::None,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => MpRecv::HostGone,
        }
    }

    /// Collects client replies, following the same contract every
    /// [`MpTransport`] implementation owes its caller — see the
    /// [`MpTransport::recv_replies`] doc comment.
    ///
    /// This used to pack replies back to back from offset zero, use a
    /// two-sided staleness test, and never wait for a reply in flight. Because
    /// [`Wifi::mp_client_reply_rx`](super::Wifi::mp_client_reply_rx) reads
    /// `mp_client_replies[(aid - 1) * 1024]`, the packed layout put every reply
    /// somewhere the hardware never looks — yet the headless harness
    /// (`core/examples/mp_loopback.rs`) read the buffer the same wrong way and
    /// so reported success while real play failed. See
    /// `docs/design/review_mp_local2.md` P1-2.
    fn recv_replies(&mut self, buf: &mut [u8], timestamp_us: u64, aid_mask: u16) -> u16 {
        let mut answered = 0u16;

        // **Deliberate deviation from the contract's release clause.** Every
        // other rule is honoured; this one drains what has already arrived and
        // never waits out [`LinkHints::recv_timeout`].
        //
        // Waiting presumes the replying peer runs on another thread that can
        // post while this one blocks -- true of `LocalMp` (a semaphore posted
        // by the other instance's thread) and of `NetTransport` (a socket RX
        // thread), but false here. A `LoopbackTransport` pair is driven from a
        // single thread by design, so a blocking wait cannot be satisfied: it
        // stalls the one thread that could have produced the reply, for the
        // full budget, on every CMD round. Adding the wait hung
        // `core/examples/mp_loopback.rs` outright.
        //
        // With no wait to release there is also nothing for melonDS's
        // "every connected instance has been heard from" condition to release,
        // so a zero-length keep-alive is simply skipped rather than tracked.
        while let Ok(frame) = self.reply_rx.try_recv() {
            // A zero-length reply names no AID. `aid == 0` would underflow the
            // slot index below, and `aid >= 16` has no slot at all.
            if frame.data.is_empty() || frame.aid == 0 || frame.aid >= 16 {
                continue;
            }
            if aid_mask & (1 << frame.aid) == 0 {
                continue;
            }

            // One-sided staleness test, following melonDS's
            // `header->Timestamp < (timestamp - 32)`: a client whose emulated
            // clock legitimately runs *ahead* of the host — which the host's own
            // ack frame grants it — must not have its reply discarded. Only a
            // lagging reply is stale. Saturating rather than wrapping, so no
            // reply is vacuously stale in the opening milliseconds of a session.
            if frame.timestamp_us + 32_000 < timestamp_us {
                continue;
            }

            // Fixed 1 KiB slot per association ID, matching melonDS's
            // `packets[(aid-1)*1024]` and what `Wifi::mp_client_reply_rx` reads
            // back.
            let slot = (frame.aid as usize - 1) * 1024;
            let end = (slot + frame.data.len()).min(buf.len()).min(slot + 1024);
            if end > slot {
                buf[slot..end].copy_from_slice(&frame.data[..end - slot]);
            }

            answered |= 1 << frame.aid;
            if answered & aid_mask == aid_mask {
                break;
            }
        }
        answered
    }

    fn link_hints(&self) -> LinkHints {
        self.hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_round_trips_between_paired_transports() {
        let (mut host, mut client) = LoopbackTransport::new_pair();
        host.send_packet(&[1, 2, 3], 100);
        let mut buf = [0u8; 16];
        let recv = client.recv_packet(&mut buf);
        assert_eq!(
            recv,
            MpRecv::Frame { len: 3, kind: MpFrameKind::Packet, timestamp_us: 100, runahead_us: 0 }
        );
        assert_eq!(&buf[..3], &[1, 2, 3]);
    }

    #[test]
    fn recv_host_packet_ignores_non_host_senders() {
        let (mut host, mut client) = LoopbackTransport::new_pair();
        // A frame from a non-host sender would never occur via `LoopbackTransport`
        // (only host/client exist), but the host-only filter inside
        // `recv_host_packet` must still hold when a genuine host frame arrives.
        host.send_cmd(&[9, 9], 50);
        let mut buf = [0u8; 16];
        let recv = client.recv_host_packet(&mut buf, Duration::from_millis(10));
        assert_eq!(
            recv,
            MpRecv::Frame { len: 2, kind: MpFrameKind::Cmd, timestamp_us: 50, runahead_us: 0 }
        );
    }

    #[test]
    fn recv_replies_collects_within_timestamp_tolerance() {
        let (mut host, mut client) = LoopbackTransport::new_pair();
        client.send_reply(&[7], 1_000, 1);
        let mut buf = [0u8; 64];
        let answered = host.recv_replies(&mut buf, 1_000, 1 << 1);
        assert_eq!(answered, 1 << 1);
    }

    /// Replies are addressed by association ID into fixed 1 KiB slots, exactly
    /// as `Wifi::mp_client_reply_rx` reads them back. Packing them back to back
    /// from offset zero — what this used to do — put every reply somewhere the
    /// hardware never looks, while the loopback-based harness read the buffer
    /// the same wrong way and reported success.
    /// `docs/design/review_mp_local2.md` P1-2.
    #[test]
    fn recv_replies_writes_each_reply_to_its_per_aid_slot() {
        let (mut host, mut client) = LoopbackTransport::new_pair();
        client.send_reply(&[0xAB; 4], 100_000, 2);

        let mut buf = [0u8; 15 * 1024];
        let answered = host.recv_replies(&mut buf, 100_000, 1 << 2);

        assert_eq!(answered, 1 << 2);
        assert_eq!(&buf[1024..1028], &[0xAB; 4], "AID 2 occupies the second 1 KiB slot");
        assert!(buf[..1024].iter().all(|&b| b == 0), "AID 1's slot must be untouched");
    }

    /// A client whose emulated clock legitimately runs ahead of the host must
    /// still have its reply accepted — the host's ack frame is what granted it
    /// that run-ahead. Only a lagging reply is stale. The previous two-sided
    /// `abs_diff` test rejected exactly the case the protocol authorises.
    #[test]
    fn recv_replies_staleness_test_is_one_sided() {
        let (mut host, mut client) = LoopbackTransport::new_pair();
        let mut buf = [0u8; 15 * 1024];

        client.send_reply(&[1], 1_100_000, 1);
        assert_eq!(
            host.recv_replies(&mut buf, 1_000_000, 1 << 1),
            1 << 1,
            "a reply 100ms ahead of the host is valid"
        );

        client.send_reply(&[1], 1_000, 1);
        assert_eq!(
            host.recv_replies(&mut buf, 1_000_000, 1 << 1),
            0,
            "a reply lagging far behind the host is stale"
        );
    }

    /// `aid == 0` would underflow the slot index and `aid >= 16` has no slot;
    /// both must be rejected without writing anything.
    #[test]
    fn recv_replies_rejects_out_of_range_association_ids() {
        let (mut host, mut client) = LoopbackTransport::new_pair();
        client.send_reply(&[0xFF; 4], 100_000, 16);

        let mut buf = [0u8; 15 * 1024];
        let answered = host.recv_replies(&mut buf, 100_000, 0xFFFF);

        assert_eq!(answered, 0, "AID 16 is out of range");
        assert!(buf.iter().all(|&b| b == 0), "nothing may be written for an invalid AID");
    }
}
