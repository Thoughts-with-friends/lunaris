//! RX path: polling the transport for inbound frames, validating the
//! 12-byte hardware header, classifying frames for the RX ring header, and
//! the MP client synchronization state machine.
//!
//! Ported from melonDS `CheckRX` (`Wifi.cpp:1564-1730`) for frame
//! acceptance and MP-sync pacing, and `FinishRX` (`Wifi.cpp:1245-1521`) for
//! the RX-ring header build and the MP-reply/beacon-sync triggers. See
//! `docs/design/local-mp-melonds-parity.md` for the gap-by-gap rationale,
//! especially §2 on why `rxflags` must be classified from raw frame bytes
//! (the melonDS MP MAC constants) rather than from the transport's
//! [`MpFrameKind`] tag.
//!
//! # Deferred header build
//! melonDS builds the RX header and raises IRQ 0 only once the simulated
//! per-byte transfer time elapses (`FinishRX`, driven by `USTimer`'s pump).
//! This module keeps that timing property while still writing the frame
//! body immediately (matching the rest of this emulator's simplification of
//! not pumping byte-by-byte): [`Wifi::start_rx`] computes the header
//! content and stores it in [`PendingRxHeader`]; [`Wifi::step_rx`] writes it
//! into RAM, advances the cursor, and raises IRQ 0 once the transfer budget
//! elapses. `rx_buffer` is guaranteed untouched in the meantime, since
//! nothing calls [`Wifi::check_rx`] again while `com_status` bit 0 is set.

use std::sync::atomic::{AtomicBool, Ordering};

use super::{
    Wifi,
    mp::{MpFrameKind, MpRecv},
    regs::*,
};
use crate::hw::interrupt_controller::InterruptRequest;

/// `03 09 BF 00 00 00` — tags a host MP command frame's address 1.
const MP_CMD_MAC: [u8; 6] = [0x03, 0x09, 0xBF, 0x00, 0x00, 0x00];
/// `03 09 BF 00 00 10` — tags a client MP reply frame's address 3.
const MP_REPLY_MAC: [u8; 6] = [0x03, 0x09, 0xBF, 0x00, 0x00, 0x10];
/// `03 09 BF 00 00 03` — tags a host MP acknowledgement frame's address 1.
const MP_ACK_MAC: [u8; 6] = [0x03, 0x09, 0xBF, 0x00, 0x00, 0x03];

fn mac_eq(buf: &[u8], offset: usize, mac: [u8; 6]) -> bool {
    buf.get(offset..offset + 6).is_some_and(|s| s == mac)
}

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

/// The RX-ring header build, computed at reception start and applied once
/// the simulated transfer time elapses. See the module-level doc comment.
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Default)]
pub(super) struct PendingRxHeader {
    /// `false` if the frame's destination address didn't match us (or
    /// wasn't multicast/broadcast): the header, cursor advance, IRQ 0, and
    /// MP-reply/beacon triggers are all skipped. Ported from `FinishRX`'s
    /// early return (`Wifi.cpp:1270-1275`).
    pub keep: bool,
    /// `true` for a CMD frame whose sequence number repeats the last one
    /// seen: the header and cursor advance are skipped, but the
    /// MP-reply/beacon triggers still fire. Ported from `FinishRX`'s
    /// `cmd_dupe` handling (`Wifi.cpp:1393-1399`, `1463`).
    pub cmd_dupe: bool,
    /// Byte address (already masked into the RX ring) the header should be
    /// written at.
    pub header_addr: u16,
    /// Classified per `docs/design/local-mp-melonds-parity.md` §3.3.
    pub rxflags: u16,
    /// Original (pre-crop) TX rate byte (`0x0A` or `0x14`).
    pub tx_rate: u8,
    /// Cropped frame length, per `W_RXLenCrop`.
    pub framelen: u16,
}

/// A frame's classification, stashed by [`Wifi::check_rx`] when it must
/// delay a client's delivery until the simulated clock reaches the frame's
/// timestamp, and consumed once [`Wifi::tick`](super::Wifi::tick) notices
/// that timestamp has arrived. Ported from the gap between melonDS's
/// `CheckRX` (which only records `RXTimestamp`/`NextSync` here, without
/// calling `StartRX`) and `USTimer`'s later, unconditional `StartRX()` call
/// once `USTimestamp >= RXTimestamp` (`Wifi.cpp:1696-1720`, `1765-1769`).
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Default)]
pub(super) struct DeferredRxParams {
    pub armed: bool,
    pub keep: bool,
    pub cmd_dupe: bool,
    pub rxflags: u16,
    pub tx_rate: u8,
    pub framelen: u16,
}

/// Advances `addr` by `inc` bytes (in 2-byte steps), wrapping from `end`
/// back to `base`. Ported from `IncrementRXAddr` (`Wifi.cpp:1206-1215`).
fn increment_rx_addr(addr: &mut u32, inc: u32, base: u32, end: u32) {
    let mut i = 0;
    while i < inc {
        *addr = (*addr + 2) & 0x1FFE;
        if *addr == end {
            *addr = base;
        }
        i += 2;
    }
}

impl Wifi {
    fn bssid(&self) -> [u8; 6] {
        let b0 = self.ioport(W_BSSID0);
        let b1 = self.ioport(W_BSSID1);
        let b2 = self.ioport(W_BSSID2);
        [b0 as u8, (b0 >> 8) as u8, b1 as u8, (b1 >> 8) as u8, b2 as u8, (b2 >> 8) as u8]
    }

    /// Classifies the frame currently staged in `rx_buffer` for the RX
    /// header's `rxflags` field, and detects a duplicate CMD frame via
    /// `mp_last_seqno`. Ported from the management/data branches of
    /// `FinishRX`'s classification switch (`Wifi.cpp:1292-1407`).
    ///
    /// Returns `None` if `W_RXFilter`/`W_RXFilter2` reject the frame outright.
    /// These are **drops**, not annotations: melonDS `return`s from `FinishRX`
    /// before writing the RX header, so a rejected frame never becomes visible
    /// to the driver at all. This port previously computed `rxflags` and never
    /// dropped, handing the driver's state machine frames it does not
    /// expect -- including control frames, which hardware discards unless they
    /// are PS-poll. See `docs/design/local-mp-melonds-parity-2.md` F7.
    ///
    /// The filter defaults (`W_RXFilter = 0x0401`, `W_RXFilter2 = 0x0008`) are
    /// installed by the `W_ModeReset` bit-14 write handled in
    /// [`super::regs`]; without that, every filter test here reads zero.
    fn classify_rxflags(&mut self) -> Option<(u16, bool)> {
        let frame_ctl = self.rx_buffer[12] as u16 | (self.rx_buffer[13] as u16) << 8;
        let mut rxflags = 0x0010u16;
        let mut cmd_dupe = false;
        let bssid = self.bssid();
        let rxfilter = self.ioport(W_RXFilter);

        match (frame_ctl >> 2) & 0x3 {
            0 => {
                // Management frame.
                if mac_eq(&self.rx_buffer, 12 + 16, bssid) {
                    rxflags |= 0x8000;
                }

                let subtype = (frame_ctl >> 4) & 0xF;
                if subtype == 0x8 {
                    // Beacon.
                    if rxflags & 0x8000 == 0 && rxfilter & (1 << 0) == 0 {
                        return None;
                    }
                    rxflags |= 0x0001;
                } else if (subtype <= 0x5 || (0xA..=0xC).contains(&subtype))
                    && rxflags & 0x8000 == 0
                    && rxfilter & (3 << 9) == 0
                {
                    return None;
                }
            }
            1 => {
                // Control frame. Hardware accepts only PS-poll here and
                // discards every other subtype (`Wifi.cpp:1327-1345`).
                if frame_ctl & 0xF0 != 0xA0 {
                    return None;
                }
                if mac_eq(&self.rx_buffer, 12 + 4, bssid) {
                    rxflags |= 0x8000;
                }
                if rxflags & 0x8000 == 0 && rxfilter & (1 << 11) == 0 {
                    return None;
                }
                rxflags |= 0x0005;
            }
            2 => {
                // Data frame.
                let fromto = ((frame_ctl >> 8) & 0x3) as usize;
                if self.ioport(W_RXFilter2) & (1 << fromto) != 0 {
                    return None;
                }

                let bssid_offset = [16usize, 4, 10, 0][fromto];
                if bssid_offset != 0 && mac_eq(&self.rx_buffer, 12 + bssid_offset, bssid) {
                    rxflags |= 0x8000;
                }

                if rxflags & 0x8000 == 0 && rxfilter & (1 << 11) == 0 {
                    return None;
                }
                // Retransmitted frame.
                if frame_ctl & (1 << 11) != 0 && rxfilter & (1 << 0) == 0 {
                    return None;
                }

                if mac_eq(&self.rx_buffer, 12 + 16, MP_REPLY_MAC) {
                    rxflags |= if frame_ctl & 0xF0 == 0x50 { 0x000F } else { 0x000E };
                } else if mac_eq(&self.rx_buffer, 12 + 4, MP_CMD_MAC) {
                    let seqno =
                        self.rx_buffer[12 + 22] as u16 | (self.rx_buffer[12 + 23] as u16) << 8;
                    if seqno == self.mp_last_seqno {
                        cmd_dupe = true;
                    }
                    self.mp_last_seqno = seqno;
                    rxflags |= 0x000C;
                } else if mac_eq(&self.rx_buffer, 12 + 4, MP_ACK_MAC) {
                    rxflags |= 0x000D;
                } else {
                    rxflags |= 0x0008;
                }

                // Per-subtype gating (`Wifi.cpp:1409-1457`).
                let accepted = match (frame_ctl >> 4) & 0xF {
                    0x0 | 0x4 => true,
                    0x1 => match rxflags & 0xF {
                        0xD => rxfilter & (1 << 7) != 0,
                        0xE => true,
                        _ => rxfilter & (1 << 1) != 0,
                    },
                    0x2 => rxflags & 0xF == 0xC || rxfilter & (1 << 2) != 0,
                    0x3 => rxfilter & (1 << 3) != 0,
                    0x5 => {
                        if rxflags & 0xF == 0xF {
                            rxfilter & (1 << 8) != 0
                        } else {
                            rxfilter & (1 << 4) != 0
                        }
                    }
                    0x6 => rxfilter & (1 << 5) != 0,
                    0x7 => rxfilter & (1 << 6) != 0,
                    _ => false,
                };
                if !accepted {
                    return None;
                }
            }
            // Frame type 3 (reserved) has no branch in melonDS's switch: it
            // keeps the base `rxflags` and is delivered.
            _ => {}
        }

        Some((rxflags, cmd_dupe))
    }

    /// Applies `W_RXLenCrop` to a frame length. WEP-frame cropping
    /// (`Wifi.cpp:1635-1639`) is not ported -- local MP play does not use
    /// WEP -- so this always takes the non-WEP branch
    /// (`Wifi.cpp:1640-1641`).
    fn crop_framelen(&self, original: u16) -> u16 {
        let crop = (self.ioport(W_RXLenCrop) << 1) & 0x1FE;
        original.saturating_sub(crop)
    }

    /// Polls the transport for one inbound frame and, if valid, starts the
    /// hardware RX byte-pump and/or updates MP sync state. Returns `true`
    /// if a frame was accepted.
    pub(super) fn check_rx(&mut self, kind: RxKind, request: &mut InterruptRequest) -> bool {
        if self.ioport(W_PowerState) & (1 << 9) != 0 {
            return false;
        }
        if self.ioport(W_RXCnt) & 0x8000 == 0 {
            self.diag.drops.rx_disabled += 1;
            if super::debug_enabled() && rx_gate_warn_latch() {
                eprintln!(
                    "[wifi] check_rx: W_RXCnt bit15 (RX enable) is clear -- driver has not \
                     armed reception yet, so no inbound frame can be delivered regardless of \
                     what's on the wire"
                );
            }
            return false;
        }
        // The RX ring hasn't been configured yet. Ported from
        // `Wifi.cpp:1572-1573`; without this guard a frame arriving before
        // the driver sets up `W_RXBufBegin`/`W_RXBufEnd` corrupts Wi-Fi RAM.
        if self.ioport(W_RXBufBegin) == self.ioport(W_RXBufEnd) {
            self.diag.drops.ring_unconfigured += 1;
            return false;
        }

        self.diag.rx_polls += 1;
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
            MpRecv::None => {
                self.diag.rx_empty += 1;
                return false;
            }
        };

        if len < 12 + 24 {
            self.diag.drops.too_short += 1;
            return false; // Too short to contain a valid 802.11 header.
        }
        let frame_len = buf[10] as usize | (buf[11] as usize) << 8;
        if frame_len != len - 12 {
            self.diag.drops.bad_length += 1;
            warn!("wifi: bad MP frame length {frame_len}/{}", len - 12);
            return false;
        }
        let channel = buf[9];
        if channel as i32 != self.cur_channel || self.cur_channel == 0 {
            self.diag.drops.channel_mismatch += 1;
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
        let tx_rate = self.rx_buffer[8];
        let mac_good = self.rx_buffer[16] & 0x01 != 0 || self.mac_matches(&self.rx_buffer[16..22]);
        let is_packet = frame_kind == MpFrameKind::Packet;

        // Classified from raw frame bytes (MP MAC constants), not from
        // `frame_kind` -- this is what the game actually reads out of the
        // RX header. See the module doc comment and
        // `docs/design/local-mp-melonds-parity.md` §2.
        let Some((rxflags, cmd_dupe)) = self.classify_rxflags() else {
            self.diag.drops.filtered += 1;
            if super::debug_enabled() {
                eprintln!(
                    "[wifi] RX dropped by W_RXFilter/W_RXFilter2 (filter=0x{:04X}/0x{:04X}, \
                     frame_ctl=0x{frame_ctl:04X})",
                    self.ioport(W_RXFilter),
                    self.ioport(W_RXFilter2)
                );
            }
            return false;
        };
        let cropped_framelen = self.crop_framelen(frame_len as u16);

        // Stage 3/4 of the `diag` summary: the frame survived every check,
        // and this is how the hardware classified it.
        self.diag.rx_accepted += 1;
        if frame_ctl & (1 << 11) != 0 {
            self.diag.rx_retry_flagged += 1;
        }
        // Authentication body: algorithm / sequence / status.
        if (frame_ctl >> 2) & 0x3 == 0 && (frame_ctl >> 4) & 0xF == 0xB {
            for (i, slot) in self.diag.last_auth.iter_mut().enumerate() {
                let o = 12 + 24 + i * 2;
                *slot = self.rx_buffer.get(o).copied().unwrap_or(0) as u16
                    | (self.rx_buffer.get(o + 1).copied().unwrap_or(0) as u16) << 8;
            }
        }
        match rxflags & 0x800F {
            0x8001 => self.diag.rxflags_beacon += 1,
            0x800C => self.diag.rxflags_cmd += 1,
            0x800D => self.diag.rxflags_ack += 1,
            0x800E | 0x800F => self.diag.rxflags_reply += 1,
            // Management frames carry no MP MAC and land here; the
            // association/authentication traffic is what matters.
            _ if (frame_ctl >> 2) & 0x3 == 0 => {
                self.diag.rxflags_mgmt += 1;
                self.diag.rx_mgmt_subtype[((frame_ctl >> 4) & 0xF) as usize] += 1;
            }
            _ => {}
        }

        if super::debug_enabled() {
            eprintln!(
                "[wifi] RX accepted: kind={frame_kind:?} frame_type=0x{frame_type:04X} \
                 mac_good={mac_good} channel={channel} is_mp_client={} len={len} \
                 rxflags=0x{rxflags:04X} cmd_dupe={cmd_dupe}",
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
                self.next_sync =
                    self.rx_timestamp + frame_time_us(cropped_framelen as usize, tx_rate);
            }
            self.rx_timestamp = 0;
            self.start_rx(request, mac_good, cmd_dupe, rxflags, tx_rate, cropped_framelen);
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
            self.start_rx(request, mac_good, cmd_dupe, rxflags, tx_rate, cropped_framelen);
        } else if mac_good && self.is_mp_client {
            // Delay delivery until our clock reaches the frame's
            // timestamp, and extend our next mandatory sync point. Ported
            // from `Wifi.cpp:1696-1720`: `StartRX` is *not* called here --
            // it fires later, from `Wifi::tick`'s `rx_timestamp` check,
            // once the simulated clock actually reaches this frame's
            // timestamp. The classification computed above must survive
            // until then, so it's stashed in `rx_deferred`.
            self.rx_timestamp = timestamp_us.max(self.us_timestamp);
            self.next_sync = self.rx_timestamp + frame_time_us(cropped_framelen as usize, tx_rate);

            match frame_kind {
                MpFrameKind::Cmd => {
                    let client_time =
                        self.rx_buffer[12 + 24] as u16 | (self.rx_buffer[12 + 25] as u16) << 8;
                    let client_mask =
                        self.rx_buffer[12 + 26] as u16 | (self.rx_buffer[12 + 27] as u16) << 8;
                    let num_clients = (1..16u16).filter(|i| client_mask & (1 << i) != 0).count();
                    self.next_sync += 112 + (client_time as u64 + 10) * num_clients as u64;
                    // The reply/blank-reply trigger itself now fires from
                    // `step_rx` at completion, keyed off `rxflags`, not
                    // here -- see Gap 3.1/3.2.
                }
                MpFrameKind::Ack => {
                    self.next_sync += runahead_us as u64;
                }
                MpFrameKind::Packet | MpFrameKind::Reply => {}
            }
            self.rx_deferred = DeferredRxParams {
                armed: true,
                keep: mac_good,
                cmd_dupe,
                rxflags,
                tx_rate,
                framelen: cropped_framelen,
            };
        } else {
            self.rx_timestamp = 0;
            self.start_rx(request, mac_good, cmd_dupe, rxflags, tx_rate, cropped_framelen);
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

    /// Reserves 12 header bytes at the current write cursor, copies
    /// `cropped_framelen` bytes of the frame body (already staged in
    /// `rx_buffer` from offset 12) into WiFi RAM past that reservation, and
    /// arms a completion budget consumed by [`Wifi::step_rx`], which builds
    /// and writes the actual header once that budget elapses. Ported from
    /// `StartRX` (`Wifi.cpp:1217-1243`); see the module doc comment for why
    /// the header write itself is deferred to [`Wifi::step_rx`].
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_rx(
        &mut self,
        request: &mut InterruptRequest,
        keep: bool,
        cmd_dupe: bool,
        rxflags: u16,
        tx_rate: u8,
        cropped_framelen: u16,
    ) {
        let base = self.ioport(W_RXBufBegin) as u32 & 0x1FFE;
        let end = self.ioport(W_RXBufEnd) as u32 & 0x1FFE;

        // `W_RXBufWriteCursor` is a *halfword*-unit address register, so the
        // byte offset is `cursor << 1`. Masking it instead (as this line used
        // to) placed the RX header at roughly half the intended offset: with
        // the cursor at zero after a `W_RXCnt` bit-0 reset the first frame
        // still landed correctly, and every frame after it landed inside the
        // previous frame's body. `step_rx` already stored the cursor back in
        // the halfword convention (`>> 1`), so the two halves of this module
        // disagreed. Ported from `StartRX` (`Wifi.cpp:1228`) and `FinishRX`
        // (`Wifi.cpp:1466`), which both shift. See
        // `docs/design/local-mp-melonds-parity-2.md` §2 and F1.
        let header_addr = ((self.ioport(W_RXBufWriteCursor) as u32) << 1) & 0x1FFE;
        let mut addr = header_addr;
        increment_rx_addr(&mut addr, 12, base, end);

        // melonDS's byte pump aborts the reception when the write cursor
        // catches up with `W_RXBufReadCursor` -- the driver has not drained
        // the previous frame yet, and continuing would overwrite unread data
        // (`Wifi.cpp:1918-1938`). This port writes the body in one step rather
        // than pumping it, so it detects the same condition up front and drops
        // the frame instead of corrupting the ring. See
        // `docs/design/local-mp-melonds-parity-2.md` F8e.
        let read_cursor = ((self.ioport(W_RXBufReadCursor) as u32) << 1) & 0x1FFE;
        let body_len = cropped_framelen as usize;
        let mut probe = addr;
        let mut overruns = false;
        let mut i = 0;
        while i < body_len {
            if probe == read_cursor {
                overruns = true;
                break;
            }
            increment_rx_addr(&mut probe, 2, base, end);
            i += 2;
        }
        if overruns {
            self.diag.drops.ring_full += 1;
            if super::debug_enabled() {
                eprintln!(
                    "[wifi] RX dropped: ring full (wr=0x{header_addr:04X} rd=0x{read_cursor:04X} \
                     len={body_len}) -- driver has not drained the previous frame"
                );
            }
            return;
        }

        let mut i = 0;
        while i < body_len {
            let a = addr as usize;
            if a < self.ram.len() {
                self.ram[a] = self.rx_buffer.get(12 + i).copied().unwrap_or(0);
            }
            if a + 1 < self.ram.len() {
                self.ram[a + 1] = self.rx_buffer.get(12 + i + 1).copied().unwrap_or(0);
            }
            increment_rx_addr(&mut addr, 2, base, end);
            i += 2;
        }
        self.set_ioport(W_RXTXAddr, (addr >> 1) as u16);

        self.rx_pending = PendingRxHeader {
            keep,
            cmd_dupe,
            header_addr: header_addr as u16,
            rxflags,
            tx_rate,
            framelen: cropped_framelen,
        };

        self.com_status |= 0x1;
        self.rx_time = cropped_framelen as i32 * if tx_rate == 0x14 { 4 } else { 8 };
        // Receiving (`Wifi.cpp:1241`).
        self.set_status(6);
        self.raise_irq(6, request);
    }

    /// Advances the RX completion budget by one 8µs timer tick; at
    /// completion, writes the RX header (unless the frame was for someone
    /// else or a duplicate CMD), advances the visible write cursor, raises
    /// **IRQ 0**, and fires the MP-reply or beacon-sync trigger. Ported
    /// from `FinishRX` (`Wifi.cpp:1245-1521`); see the module doc comment.
    pub(super) fn step_rx(&mut self, request: &mut InterruptRequest) {
        if self.com_status & 0x1 == 0 {
            return;
        }
        self.rx_time -= Wifi::TIMER_INTERVAL_US as i32;
        if self.rx_time > 0 {
            return;
        }
        self.com_status &= !0x1;
        self.rx_counter = 0;
        if self.com_status == 0 {
            // Back to idle (`Wifi.cpp:1255-1258`).
            self.set_status(1);
        }

        let pending = self.rx_pending;
        self.rx_pending = PendingRxHeader::default();

        if !pending.keep {
            return;
        }

        if !pending.cmd_dupe {
            let header_addr = pending.header_addr as usize;
            let write16 = |ram: &mut [u8], off: usize, value: u16| {
                if off + 1 < ram.len() {
                    ram[off] = value as u8;
                    ram[off + 1] = (value >> 8) as u8;
                }
            };
            write16(&mut self.ram, header_addr, pending.rxflags);
            write16(&mut self.ram, header_addr + 2, 0x0040);
            write16(&mut self.ram, header_addr + 6, u16::from(pending.tx_rate));
            write16(&mut self.ram, header_addr + 8, pending.framelen);
            write16(&mut self.ram, header_addr + 0xA, 0x4080);

            let base = self.ioport(W_RXBufBegin) as u32 & 0x1FFE;
            let end = self.ioport(W_RXBufEnd) as u32 & 0x1FFE;
            let mut cursor = u32::from(self.ioport(W_RXTXAddr)) << 1;
            if cursor & 0x2 != 0 {
                increment_rx_addr(&mut cursor, 2, base, end);
            }
            self.set_ioport(W_RXBufWriteCursor, ((cursor & !0x3) >> 1) as u16);

            self.raise_irq(0, request);
        }

        match pending.rxflags & 0x800F {
            0x800C => {
                // Reply to a CMD frame addressed to our BSSID. Ported from
                // `Wifi.cpp:1487-1504`. See
                // `docs/design/local-mp-melonds-parity.md` Gap 3.1/3.2.
                let clienttime =
                    self.rx_buffer[12 + 24] as u16 | (self.rx_buffer[12 + 25] as u16) << 8;
                let clientmask =
                    self.rx_buffer[12 + 26] as u16 | (self.rx_buffer[12 + 27] as u16) << 8;
                let our_aid = self.ioport(W_AIDLow);
                if our_aid != 0 && clientmask & (1 << our_aid) != 0 {
                    self.start_mp_reply(clienttime, clientmask);
                } else if let Some(mut transport) = self.transport.take() {
                    // Blank keep-alive reply: lets the host avoid a full
                    // timeout even though this client had nothing to send.
                    transport.send_reply(&[], self.us_timestamp, 0);
                    self.transport = Some(transport);
                }
            }
            0x8001 => {
                // Beacon with our BSSID: adopt its timestamp into the MP
                // sync clock. Ported from `Wifi.cpp:1506-1520`; see
                // `docs/design/local-mp-melonds-parity.md` Gap 4.3.
                let mut len =
                    u32::from(pending.framelen) * if pending.tx_rate == 0x14 { 4 } else { 8 };
                len = len.saturating_sub(76);
                let mut ts_bytes = [0u8; 8];
                if let Some(src) = self.rx_buffer.get(12 + 24..12 + 32) {
                    ts_bytes.copy_from_slice(src);
                }
                let timestamp = u64::from_le_bytes(ts_bytes);
                self.us_counter = timestamp.wrapping_add(u64::from(len));
            }
            _ => {}
        }
    }

    /// Feeds one queued client reply back through the RX path so the
    /// host's own game receives it as an ordinary frame. Ported from
    /// `MPClientReplyRX` (`Wifi.cpp:1523-1562`); see
    /// `docs/design/local-mp-melonds-parity.md` Gap 3.3 -- without this,
    /// the host never delivers any client's reply data to the game, even
    /// though the transport successfully collected it.
    pub(super) fn mp_client_reply_rx(&mut self, client: u16, request: &mut InterruptRequest) {
        if self.ioport(W_PowerState) & (1 << 9) != 0 {
            return;
        }
        if self.ioport(W_RXCnt) & 0x8000 == 0 {
            return;
        }
        if self.ioport(W_RXBufBegin) == self.ioport(W_RXBufEnd) {
            return;
        }

        let slot_off = (client as usize - 1) * 1024;
        let Some(reply) = self.mp_client_replies.get(slot_off..slot_off + 1024) else { return };
        let mut framelen = reply[10] as u16 | (reply[11] as u16) << 8;
        let tx_rate = reply[8];

        // WEP-frame cropping (`Wifi.cpp:1544-1548`) is not ported -- see
        // `Wifi::crop_framelen`.
        let crop = (self.ioport(W_RXLenCrop) << 1) & 0x1FE;
        framelen = framelen.saturating_sub(crop);

        let total = (12 + framelen as usize).min(reply.len()).min(self.rx_buffer.len());
        self.rx_buffer[..total].copy_from_slice(&reply[..total]);

        let Some((rxflags, cmd_dupe)) = self.classify_rxflags() else { return };
        let mac_good = self.rx_buffer[16] & 0x01 != 0 || self.mac_matches(&self.rx_buffer[16..22]);

        self.rx_timestamp = 0;
        self.start_rx(request, mac_good, cmd_dupe, rxflags, tx_rate, framelen);
    }
}

fn frame_time_us(frame_len: usize, tx_rate: u8) -> u64 {
    frame_len as u64 * if tx_rate == 0x14 { 4 } else { 8 }
}
