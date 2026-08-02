//! RX path: polling the transport for inbound frames, validating the
//! 12-byte hardware header, and the MP client synchronization state
//! machine. Ported from melonDS `CheckRX`
//! (`docs/design/melonds/WiFi.cpp:1558-1724`); see
//! `docs/design/design_lan.md` §6.7 for the four-branch sync table this
//! implements.

use std::sync::atomic::{AtomicBool, Ordering};

use super::{
    Wifi,
    mp::{MpFrameKind, MpRecv},
    regs::*,
};
use crate::hw::interrupt_controller::InterruptRequest;

/// One-shot latch so the "RX not armed" diagnostic (checked every 8µs
/// tick while `LUNARIS_WIFI_DEBUG` is set) prints once instead of
/// flooding the terminal for as long as the driver leaves `W_RXCnt`
/// disabled.
fn rx_gate_warn_latch() -> bool {
    static WARNED: AtomicBool = AtomicBool::new(false);
    !WARNED.swap(true, Ordering::Relaxed)
}

/// Which RX source to poll.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RxKind {
    /// Regular traffic (beacons, data, association).
    Regular,
    /// Host frames only, while acting as an MP client past its sync point.
    HostFrames,
}

impl Wifi {
    /// Polls the transport for one inbound frame and, if valid, starts the
    /// hardware RX byte-pump or updates MP sync state. Returns `true` if a
    /// frame was accepted.
    pub(super) fn check_rx(&mut self, kind: RxKind, request: &mut InterruptRequest) -> bool {
        if self.ioport(W_PowerState) & (1 << 9) != 0 {
            return false;
        }
        if self.ioport(W_RXCnt) & 0x8000 == 0 {
            if super::debug_enabled() && rx_gate_warn_latch() {
                eprintln!(
                    "[wifi] check_rx: W_RXCnt bit15 (RX enable) is clear -- driver has not \
                     armed reception yet, so no inbound frame can be delivered regardless of \
                     what's on the wire"
                );
            }
            return false;
        }

        let Some(mut transport) = self.transport.take() else { return false };
        let mut buf = vec![0u8; Wifi::RX_BUFFER_SIZE];
        let recv = match kind {
            RxKind::Regular => transport.recv_packet(&mut buf),
            RxKind::HostFrames => {
                transport.recv_host_packet(&mut buf, self.link_hints().recv_timeout)
            }
        };
        self.transport = Some(transport);

        let (len, frame_kind, timestamp_us, runahead_us) = match recv {
            MpRecv::Frame { len, kind, timestamp_us, runahead_us } => {
                (len, kind, timestamp_us, runahead_us)
            }
            MpRecv::HostGone => {
                self.is_mp = false;
                self.is_mp_client = false;
                return false;
            }
            MpRecv::None => return false,
        };

        if len < 12 + 24 {
            return false; // Too short to contain a valid 802.11 header.
        }
        let frame_len = buf[10] as usize | (buf[11] as usize) << 8;
        if frame_len != len - 12 {
            warn!("wifi: bad MP frame length {frame_len}/{}", len - 12);
            return false;
        }
        let channel = buf[9];
        if channel as i32 != self.cur_channel || self.cur_channel == 0 {
            if super::debug_enabled() {
                eprintln!(
                    "[wifi] RX dropped: channel mismatch (frame channel={channel}, our \
                     cur_channel={}) -- both peers must resolve the same channel from their \
                     (possibly independently-generated) firmware RF calibration table",
                    self.cur_channel
                );
            }
            return false;
        }

        self.rx_buffer[..len].copy_from_slice(&buf[..len]);
        let frame_ctl = self.rx_buffer[12] as u16 | (self.rx_buffer[13] as u16) << 8;
        let frame_type = frame_ctl & 0x00FF;
        let mac_good = self.rx_buffer[16] & 0x01 != 0 || self.mac_matches(&self.rx_buffer[16..22]);
        let is_packet = frame_kind == MpFrameKind::Packet;

        if super::debug_enabled() {
            eprintln!(
                "[wifi] RX accepted: kind={frame_kind:?} frame_type=0x{frame_type:04X} \
                 mac_good={mac_good} channel={channel} is_mp_client={} len={len}",
                self.is_mp_client
            );
        }

        // Extend the post-beacon window on auth/assoc/data frames so a
        // laggy handshake still completes instead of timing out.
        if is_packet
            && matches!(frame_type, 0x00B0 | 0x0010 | 0x0000)
            && timestamp_us != 0
            && mac_good
        {
            let count2 = self.ioport(W_BeaconCount2);
            if count2 != 0 {
                self.set_ioport(W_BeaconCount2, count2.wrapping_add(10));
            }
        }

        if is_packet && frame_type == 0x0010 && timestamp_us != 0 && mac_good {
            // Association response: adopt the host's clock and become an
            // MP client.
            let aid =
                self.rx_buffer[12 + 24 + 4] as u16 | (self.rx_buffer[12 + 24 + 5] as u16) << 8;
            if aid != 0 {
                self.is_mp = true;
                self.is_mp_client = true;
                self.us_timestamp = timestamp_us;
                self.next_sync = self.rx_timestamp + frame_time_us(frame_len, self.rx_buffer[8]);
            }
            self.rx_timestamp = 0;
            self.start_rx(request);
        } else if is_packet
            && frame_type == 0x00C0
            && timestamp_us != 0
            && mac_good
            && self.is_mp_client
        {
            self.is_mp = false;
            self.is_mp_client = false;
            self.next_sync = 0;
            self.rx_timestamp = 0;
            self.start_rx(request);
        } else if mac_good && self.is_mp_client {
            // Delay delivery until our clock reaches the frame's
            // timestamp, and extend our next mandatory sync point.
            self.rx_timestamp = timestamp_us.max(self.us_timestamp);
            self.next_sync = self.rx_timestamp + frame_time_us(frame_len, self.rx_buffer[8]);

            match frame_kind {
                MpFrameKind::Cmd => {
                    let client_time =
                        self.rx_buffer[12 + 24] as u16 | (self.rx_buffer[12 + 25] as u16) << 8;
                    let client_mask =
                        self.rx_buffer[12 + 26] as u16 | (self.rx_buffer[12 + 27] as u16) << 8;
                    let num_clients = client_mask.count_ones();
                    self.next_sync += 112 + (client_time as u64 + 10) * num_clients as u64;

                    // If this command addresses our association id, arm
                    // the automatic MP reply slot (5). See
                    // `docs/design/melonds/WiFi.cpp:800-826`.
                    let our_aid = self.ioport(W_AIDLow) & 0xF;
                    if our_aid != 0 && client_mask & (1 << our_aid) != 0 {
                        let rate = if self.rx_buffer[8] == 0x14 { 2 } else { 1 };
                        self.start_mp_reply(rate);
                    }
                }
                MpFrameKind::Ack => {
                    self.next_sync += runahead_us as u64;
                }
                MpFrameKind::Packet | MpFrameKind::Reply => {}
            }
        } else {
            self.rx_timestamp = 0;
            self.start_rx(request);
        }

        true
    }

    fn mac_matches(&self, addr: &[u8]) -> bool {
        let mac = [
            self.ioport(W_MACAddr0) as u8,
            (self.ioport(W_MACAddr0) >> 8) as u8,
            self.ioport(W_MACAddr1) as u8,
            (self.ioport(W_MACAddr1) >> 8) as u8,
            self.ioport(W_MACAddr2) as u8,
            (self.ioport(W_MACAddr2) >> 8) as u8,
        ];
        addr.len() >= 6 && addr[..6] == mac
    }

    /// Copies the frame currently staged in `rx_buffer` (12-byte hardware
    /// header + 802.11 body) into WiFi RAM at the current write cursor, and
    /// arms a completion budget consumed by [`Wifi::step_rx`].
    ///
    /// Simplified from melonDS's per-halfword-timed pump
    /// (`docs/design/melonds/WiFi.cpp:1890-1930`): the whole frame is
    /// written in one step rather than two bytes per 8µs tick, since only
    /// the total transfer budget (and the resulting RX-complete interrupt
    /// timing) matters for MP handshake correctness, not intra-frame
    /// timing. See `docs/design/design_lan.md` §6.7.
    pub(super) fn start_rx(&mut self, request: &mut InterruptRequest) {
        let frame_len = self.rx_buffer[10] as usize | (self.rx_buffer[11] as usize) << 8;
        let total = (12 + frame_len).min(self.rx_buffer.len());

        let base = self.ioport(W_RXBufBegin) as u32 & 0x1FFE;
        let end = self.ioport(W_RXBufEnd) as u32 & 0x1FFE;
        let mut addr = self.ioport(W_RXBufWriteAddr) as u32 & 0x1FFE;
        for chunk in self.rx_buffer[..total].chunks(2) {
            let lo = chunk[0];
            let hi = *chunk.get(1).unwrap_or(&0);
            let a = addr as usize;
            if a + 1 < self.ram.len() {
                self.ram[a] = lo;
                self.ram[a + 1] = hi;
            }
            addr += 2;
            if end > base && addr >= end {
                addr = base;
            }
        }
        self.set_ioport(W_RXBufWriteAddr, addr as u16 & 0x1FFE);

        self.com_status |= 0x1;
        self.rx_time = total as i32 * 2;
        self.raise_irq(11, request);
    }

    /// Advances the RX completion budget by one 8µs timer tick; fires the
    /// RX-complete and RX-count-up interrupts once it elapses.
    pub(super) fn step_rx(&mut self, request: &mut InterruptRequest) {
        if self.com_status & 0x1 == 0 {
            return;
        }
        self.rx_time -= Wifi::TIMER_INTERVAL_US as i32;
        if self.rx_time > 0 {
            return;
        }
        self.com_status &= !0x1;
        self.raise_irq(0, request);
        self.raise_irq(6, request);
    }
}

fn frame_time_us(frame_len: usize, tx_rate: u8) -> u64 {
    frame_len as u64 * if tx_rate == 0x14 { 4 } else { 8 }
}
