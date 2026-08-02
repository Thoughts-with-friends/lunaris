//! Adapter letting any [`MpInterface`] drive the Wi-Fi hardware.
//!
//! Not part of melonDS. melonDS's Wi-Fi code calls
//! `MPInterface::Get().SendCmd(inst, ...)` directly, so no adapter is
//! needed there. lunaris's `crate::hw::wifi` instead talks to
//! [`MpTransport`], a per-instance trait with no `inst` parameter — this
//! type closes that gap by pinning one instance index.
//!
//! # Semantic gaps, and how they are handled
//! * [`MpTransport::send_ack`] carries a run-ahead window; melonDS's MP
//!   interface has no such field, so it is dropped on the way out. Local
//!   MP needs no pacing (both instances share a machine), and a
//!   socket-backed backend is expected to implement [`MpTransport`]
//!   directly rather than going through here.
//! * [`MpTransport::link_hints`] is answered from a value held here and
//!   settable with [`MpInterfaceTransport::set_link_hints`], since an
//!   [`MpInterface`] publishes no pacing information of its own.
//! * [`MpTransport::recv_host_packet`]'s per-call timeout is applied by
//!   writing it to the backend's [`MpInterface::set_recv_timeout`] before
//!   each receive, which is how melonDS's `RecvTimeout` is meant to be
//!   used.

use std::time::Duration;

use super::mp_interface::{MpFrameCategory, MpInterface, MpRecvResult};
use crate::hw::{LinkHints, MpFrameKind, MpRecv, MpTransport};

/// Presents an [`MpInterface`] backend as the single-instance
/// [`MpTransport`] that `crate::hw::wifi` consumes.
pub struct MpInterfaceTransport<I: MpInterface> {
    interface: I,
    inst: u8,
    hints: LinkHints,
}

impl<I: MpInterface> MpInterfaceTransport<I> {
    /// Binds `interface` to instance index `inst`.
    #[must_use]
    pub fn new(interface: I, inst: u8) -> Self {
        MpInterfaceTransport { interface, inst, hints: LinkHints::default() }
    }

    /// The instance index every call is issued under.
    #[must_use]
    pub const fn instance(&self) -> u8 {
        self.inst
    }

    /// The wrapped backend.
    pub const fn interface(&self) -> &I {
        &self.interface
    }

    /// Mutable access to the wrapped backend.
    pub const fn interface_mut(&mut self) -> &mut I {
        &mut self.interface
    }

    /// Overrides the link parameters reported by
    /// [`MpTransport::link_hints`].
    pub const fn set_link_hints(&mut self, hints: LinkHints) {
        self.hints = hints;
    }
}

/// Translates a received frame into the Wi-Fi hardware's view of it.
/// A category the backend could not decode is reported as a regular
/// frame, which is what melonDS's Wi-Fi code assumes by default.
fn to_recv(result: MpRecvResult) -> MpRecv {
    match result {
        MpRecvResult::None => MpRecv::None,
        MpRecvResult::HostGone => MpRecv::HostGone,
        MpRecvResult::Frame { len, frame_type, timestamp } => {
            let kind = match frame_type.category() {
                Some(MpFrameCategory::Cmd) => MpFrameKind::Cmd,
                Some(MpFrameCategory::Reply) => MpFrameKind::Reply,
                Some(MpFrameCategory::Ack) => MpFrameKind::Ack,
                Some(MpFrameCategory::Regular) | None => MpFrameKind::Packet,
            };
            MpRecv::Frame { len, kind, timestamp_us: timestamp, runahead_us: 0 }
        }
    }
}

impl<I: MpInterface> MpTransport for MpInterfaceTransport<I> {
    fn begin(&mut self) {
        self.interface.begin(self.inst);
    }

    fn end(&mut self) {
        self.interface.end(self.inst);
    }

    fn send_packet(&mut self, data: &[u8], timestamp_us: u64) -> usize {
        self.interface.send_packet(self.inst, data, timestamp_us)
    }

    fn send_cmd(&mut self, data: &[u8], timestamp_us: u64) -> usize {
        self.interface.send_cmd(self.inst, data, timestamp_us)
    }

    fn send_reply(&mut self, data: &[u8], timestamp_us: u64, aid: u16) -> usize {
        self.interface.send_reply(self.inst, data, timestamp_us, aid)
    }

    fn send_ack(&mut self, data: &[u8], timestamp_us: u64, _runahead_us: u32) -> usize {
        self.interface.send_ack(self.inst, data, timestamp_us)
    }

    fn recv_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        to_recv(self.interface.recv_packet(self.inst, buf))
    }

    fn recv_host_packet(&mut self, buf: &mut [u8], timeout: Duration) -> MpRecv {
        let previous = self.interface.recv_timeout();
        self.interface.set_recv_timeout(timeout);
        let result = self.interface.recv_host_packet(self.inst, buf);
        self.interface.set_recv_timeout(previous);
        to_recv(result)
    }

    fn recv_replies(&mut self, buf: &mut [u8], timestamp_us: u64, aid_mask: u16) -> u16 {
        self.interface.recv_replies(self.inst, buf, timestamp_us, aid_mask)
    }

    fn link_hints(&self) -> LinkHints {
        self.hints
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::hw::net::local::{LocalMp, LocalMpHub};

    /// Two `MpTransport`s over one local hub, as a frontend would build
    /// them for two instances in one process.
    fn transport_pair() -> (MpInterfaceTransport<LocalMp>, MpInterfaceTransport<LocalMp>) {
        let hub = Arc::new(LocalMpHub::new());
        let host = MpInterfaceTransport::new(LocalMp::from_hub(Arc::clone(&hub)), 0);
        let client = MpInterfaceTransport::new(LocalMp::from_hub(hub), 1);
        (host, client)
    }

    #[test]
    fn packet_round_trips_and_keeps_its_kind() {
        let (mut host, mut client) = transport_pair();
        host.begin();
        client.begin();

        host.send_packet(&[1, 2, 3], 100);
        let mut buf = [0u8; 16];
        assert_eq!(
            client.recv_packet(&mut buf),
            MpRecv::Frame { len: 3, kind: MpFrameKind::Packet, timestamp_us: 100, runahead_us: 0 }
        );
        assert_eq!(&buf[..3], &[1, 2, 3]);
    }

    #[test]
    fn cmd_frames_are_reported_as_cmd() {
        let (mut host, mut client) = transport_pair();
        host.begin();
        client.begin();

        host.send_cmd(&[9, 9], 50);
        let mut buf = [0u8; 16];
        assert_eq!(
            client.recv_host_packet(&mut buf, Duration::from_millis(50)),
            MpRecv::Frame { len: 2, kind: MpFrameKind::Cmd, timestamp_us: 50, runahead_us: 0 }
        );
    }

    #[test]
    fn replies_land_in_their_aid_slot() {
        let (mut host, mut client) = transport_pair();
        host.begin();
        client.begin();

        host.send_cmd(&[0], 1_000);
        let mut buf = [0u8; 16];
        client.recv_host_packet(&mut buf, Duration::from_millis(50));
        client.send_reply(&[0xAB; 4], 1_000, 2);

        // 15 KiB, matching the hardware's `mp_client_replies` buffer.
        let mut replies = vec![0u8; 15 * 1024];
        assert_eq!(host.recv_replies(&mut replies, 1_000, 1 << 2), 1 << 2);
        // aid 2 occupies the second 1 KiB slot.
        assert_eq!(&replies[1024..1028], &[0xAB; 4]);
    }

    #[test]
    fn recv_host_packet_restores_the_backend_timeout() {
        let (mut host, mut client) = transport_pair();
        host.begin();
        client.begin();
        let before = client.interface().recv_timeout();

        let mut buf = [0u8; 16];
        client.recv_host_packet(&mut buf, Duration::from_millis(1));
        assert_eq!(client.interface().recv_timeout(), before);
    }
}
