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

    /// Host-only: collects reply frames matching `timestamp_us` (within a
    /// tolerance window) from the clients named in `aid_mask`. Returns the
    /// bitmask of AIDs that replied.
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

    fn recv_replies(&mut self, buf: &mut [u8], timestamp_us: u64, aid_mask: u16) -> u16 {
        let mut answered = 0u16;
        let mut offset = 0usize;
        // melonDS releases the reply wait on *either* "every addressed AID
        // sent data" or "every connected instance has been heard from"
        // (`docs/design/melonds/net/LocalMP.cpp:295-360`). A `LoopbackTransport`
        // pair has exactly one peer, so hearing from it at all is the second
        // condition. Tracked so a zero-length keep-alive reply -- which names
        // no AID and sets no `answered` bit -- still ends the wait, matching
        // `NetTransport::recv_replies`. See
        // `docs/design/local-mp-melonds-parity-2.md` F5.
        let mut heard_from_peer = false;

        while let Ok(frame) = self.reply_rx.try_recv() {
            heard_from_peer |= frame.sender_id != self.peer_id;

            // A zero-length reply carries no AID; its only job is the
            // `heard_from_peer` bookkeeping above.
            if frame.data.is_empty() {
                if heard_from_peer {
                    break;
                }
                continue;
            }
            if aid_mask & (1 << frame.aid) == 0 {
                continue;
            }

            // Tolerate replies from the same logical exchange
            // (within a coarse window), mirroring melonDS's ±32ms
            // reply-collection tolerance.
            if frame.timestamp_us.abs_diff(timestamp_us) > 32_000 {
                continue;
            }

            let end = (offset + frame.data.len()).min(buf.len());
            if end > offset {
                buf[offset..end].copy_from_slice(&frame.data[..end - offset]);
                offset = end;
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
}
