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
///
/// Deliberately carries no `rxflags`/`cmd_dupe`: frame classification and the
/// `W_RXFilter`/`W_RXFilter2` drop decisions happen at *completion*, in
/// [`Wifi::step_rx`], exactly as melonDS runs them in `FinishRX`. See
/// `docs/design/review_mp_local2.md` P0-1.
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Default)]
pub(super) struct PendingRxHeader {
    /// `false` if the frame's destination address didn't match us (or
    /// wasn't multicast/broadcast): the header, cursor advance, IRQ 0, and
    /// MP-reply/beacon triggers are all skipped. Ported from `FinishRX`'s
    /// early return (`Wifi.cpp:1270-1275`).
    pub keep: bool,
    /// Byte address (already masked into the RX ring) the header should be
    /// written at.
    pub header_addr: u16,
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
///
/// Like [`PendingRxHeader`], this carries no classification: the frame is
/// classified once, at completion, in [`Wifi::step_rx`].
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Default)]
pub(super) struct DeferredRxParams {
    pub armed: bool,
    pub keep: bool,
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

    /// Applies `W_RXLenCrop` to a frame length, and for a WEP frame also
    /// slides the 802.11 body down over the 4-byte WEP IV. Ported from
    /// `CheckRX` (`Wifi.cpp:1633-1643`).
    ///
    /// The WEP branch used to be skipped here on the assumption that local
    /// play never sets frame-control bit 14. When a game *does* set it, the
    /// body stays 4 bytes further along than the driver expects: the frame
    /// passes every length and filter check and is delivered intact, yet
    /// every field the driver reads out of it is shifted, so a handshake can
    /// repeat forever without ever completing.
    pub(super) fn crop_framelen(&mut self, original: u16, frame_ctl: u16) -> u16 {
        let crop = self.ioport(W_RXLenCrop);
        if frame_ctl & (1 << 14) != 0 {
            let framelen = original.saturating_sub((crop >> 7) & 0x1FE);
            if framelen > 24 {
                // `memmove(&RXBuffer[12+24], &RXBuffer[12+28], framelen)`.
                let src = 12 + 28;
                let dst = 12 + 24;
                let len = (framelen as usize).min(self.rx_buffer.len().saturating_sub(src));
                self.rx_buffer.copy_within(src..src + len, dst);
            }
            framelen
        } else {
            original.saturating_sub((crop << 1) & 0x1FE)
        }
    }

    /// Polls the transport for one inbound frame and, if valid, starts the
    /// hardware RX byte-pump and/or updates MP sync state. Returns `true`
    /// if a frame was accepted.
    pub(super) fn check_rx(&mut self, kind: RxKind, request: &mut InterruptRequest) -> bool {
        // melonDS aborts here when `W_PowerState` bit 9 reports the
        // transceiver powered down (`Wifi.cpp:1566-1567`).
        //
        // **Deliberate deviation:** this port does not, because it has no
        // reliable way back out of that state. melonDS's radio always
        // recovers -- it models `IOPORT(0x27C)`, the partial
        // `W_PowerDownCtrl` states, and a driver that drives `W_PowerState`
        // in `W_ModeWEP` mode 3 -- whereas here, measured against a real
        // game, an instance that took this branch simply stopped receiving
        // for the rest of the session: reception froze mid-session and the
        // link dropped, with the driver polling `W_PowerState` tens of
        // millions of times waiting for a wake-up that never came.
        //
        // The power state itself is still modelled and still reported to the
        // driver through `W_PowerState`/`W_TRXPower`/`W_RFStatus`; only this
        // one hard gate on the receive path is dropped, so a stalled
        // power-down can no longer take the link with it.
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
        let mut buf = vec![0u8; Wifi::RX_BUFFER_SIZE];

        // melonDS's `CheckRX` validation section is a `for (;;)` loop: a frame
        // that is too short, whose length field disagrees, or that arrived on
        // another channel is skipped with `continue`, pulling the *next* frame
        // off the transport within the same call (`Wifi.cpp:1581-1645`). Only
        // "nothing available" ends the call.
        //
        // Returning on the first rejection -- what this used to do -- cost an
        // entire polling opportunity per bad frame. On the regular path that is
        // one opportunity per 512µs; on the client's host-frame path each miss
        // additionally burns a full blocking `recv_host_packet` timeout. During
        // association the peers exchange beacons on possibly-mismatched
        // channels while the client races a `W_BeaconCount2` timeout, so
        // rejections arrive in runs and the handshake stalls. See
        // `docs/design/review_mp_local2.md` P0-2.
        //
        // The iteration bound has no melonDS counterpart (melonDS's transports
        // are in-process and drain quickly); it exists so a peer flooding
        // invalid frames cannot hold the 8µs tick indefinitely.
        //
        // It bounds *iterations*, not wall-clock cost. On [`RxKind::HostFrames`]
        // every iteration re-enters `recv_host_packet`, which blocks for up to
        // `link_hints().recv_timeout`; invalid frames that trickle in one at a
        // time therefore cost that timeout each, up to 32 times, inside a single
        // tick. melonDS's loop has exactly this shape, so the behaviour is
        // faithful rather than accidental -- but note that only a frame that
        // *arrived and was rejected* iterates. "Nothing available" returns
        // immediately, which is the common case.
        const MAX_DRAIN_PER_CALL: u32 = 32;
        let mut drained = 0u32;
        let (len, frame_kind, timestamp_us) = loop {
            if drained >= MAX_DRAIN_PER_CALL {
                return false;
            }
            drained += 1;

            let Some(mut transport) = self.transport.take() else { return false };
            let recv = match kind {
                RxKind::Regular => transport.recv_packet(&mut buf),
                RxKind::HostFrames => {
                    transport.recv_host_packet(&mut buf, self.link_hints().recv_timeout)
                }
            };
            self.transport = Some(transport);

            let (len, frame_kind, timestamp_us) = match recv {
                MpRecv::Frame { len, kind, timestamp_us, .. } => (len, kind, timestamp_us),
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
                // Too short to contain a valid 802.11 header.
                self.diag.drops.too_short += 1;
                continue;
            }
            let frame_len = buf[10] as usize | (buf[11] as usize) << 8;
            if frame_len != len - 12 {
                self.diag.drops.bad_length += 1;
                warn!("wifi: bad MP frame length {frame_len}/{}", len - 12);
                continue;
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
                continue;
            }

            // Ignore MP traffic while not engaged in an MP session. Ported
            // verbatim from `Wifi.cpp:1620-1628`, including its test of
            // `MPReplyMAC` at *both* address 3 (`+16`) and address 1 (`+4`).
            //
            // Without it a non-associated instance accepts another session's
            // CMD frames, writes them into its RX ring, and -- via
            // `Wifi::step_rx`'s `0x800C` branch -- transmits blank replies into
            // an exchange it is not part of, disrupting the peers that are.
            // See `docs/design/review_mp_local2.md` P0-5.
            if kind == RxKind::Regular
                && !self.is_mp
                && (mac_eq(&buf, 12 + 16, MP_REPLY_MAC)
                    || mac_eq(&buf, 12 + 4, MP_CMD_MAC)
                    || mac_eq(&buf, 12 + 4, MP_REPLY_MAC))
            {
                self.diag.drops.foreign_mp += 1;
                continue;
            }

            break (len, frame_kind, timestamp_us);
        };

        self.rx_buffer[..len].copy_from_slice(&buf[..len]);
        let frame_len = len - 12;
        let frame_ctl = self.rx_buffer[12] as u16 | (self.rx_buffer[13] as u16) << 8;
        let frame_type = frame_ctl & 0x00FF;
        let tx_rate = self.rx_buffer[8];
        let mac_good = self.rx_buffer[16] & 0x01 != 0 || self.mac_matches(&self.rx_buffer[16..22]);

        // `W_RXFilter`/`W_RXFilter2` are **not** consulted here. melonDS
        // classifies and filters in `FinishRX`, after the simulated transfer
        // time has elapsed; this port now does the same from
        // [`Wifi::step_rx`]. Filtering here instead had two effects, both
        // client-side: a filtered frame never reached the `next_sync` update
        // below, so the MP clock stopped gating and `Wifi::tick` re-entered
        // this function every tick; and `W_BSSID`/`W_RXFilter` were sampled at
        // frame *arrival* rather than frame *completion*, dropping association
        // traffic that raced the driver programming those registers. See
        // `docs/design/review_mp_local2.md` P0-1.
        let cropped_framelen = self.crop_framelen(frame_len as u16, frame_ctl);

        // Stage 3/4 of the `diag` summary: the frame survived every check
        // `check_rx` performs. Classification counters are bumped at
        // completion instead, in [`Wifi::step_rx`].
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

        if super::debug_enabled() {
            eprintln!(
                "[wifi] RX accepted: kind={frame_kind:?} frame_type=0x{frame_type:04X} \
                 mac_good={mac_good} is_mp_client={} len={len}",
                self.is_mp_client
            );
        }

        // Extend the post-beacon window on auth/assoc/data frames so a
        // laggy handshake still completes instead of timing out
        // (`Wifi.cpp:1660-1667`). melonDS gates this on the frame type, the
        // timestamp and `macgood` only; the transport's frame tag is
        // deliberately not consulted -- see the MP dispatch below.
        if matches!(frame_type, 0x00B0 | 0x0010 | 0x0000) && timestamp_us != 0 && mac_good {
            let count2 = self.ioport(W_BeaconCount2);
            if count2 != 0 {
                self.set_ioport(W_BeaconCount2, count2.wrapping_add(10));
            }
        }

        if frame_type == 0x0010 {
            // Record the guards before applying them: an association
            // response that arrives but leaves `is_mp` false is otherwise
            // indistinguishable from one that never came.
            self.diag.last_assoc_aid = self.rx_buffer.get(12 + 24 + 4).copied().unwrap_or(0) as u16
                | (self.rx_buffer.get(12 + 24 + 5).copied().unwrap_or(0) as u16) << 8;
            self.diag.last_assoc_mac_good = mac_good;
            self.diag.last_assoc_is_packet = frame_kind == MpFrameKind::Packet;
            self.diag.last_assoc_timestamp = timestamp_us;
        }
        if frame_type == 0x0010 && timestamp_us != 0 && mac_good {
            // Association response: adopt the host's clock and become an
            // MP client.
            let aid =
                self.rx_buffer[12 + 24 + 4] as u16 | (self.rx_buffer[12 + 24 + 5] as u16) << 8;
            if aid != 0 {
                // A single, greppable marker so a `LUNARIS_WIFI_DEBUG=1`
                // capture can be windowed around the exact moment this
                // instance became an MP client -- the register traffic just
                // after this is what decides whether the driver accepts the
                // association or re-initialises.
                if super::debug_enabled() {
                    eprintln!(
                        "[wifi] ===== ASSOC-PROMOTE aid=0x{aid:04X} us_timestamp={} =====",
                        self.us_timestamp
                    );
                }
                self.is_mp = true;
                self.is_mp_client = true;
                self.us_timestamp = timestamp_us;
                self.next_sync =
                    self.rx_timestamp + frame_time_us(cropped_framelen as usize, tx_rate);
            }
            self.rx_timestamp = 0;
            self.start_rx(request, mac_good, tx_rate, cropped_framelen);
        } else if frame_type == 0x00C0 && timestamp_us != 0 && mac_good && self.is_mp_client {
            self.is_mp = false;
            self.is_mp_client = false;
            self.next_sync = 0;
            self.rx_timestamp = 0;
            self.start_rx(request, mac_good, tx_rate, cropped_framelen);
        } else if mac_good && self.is_mp_client {
            // Delay delivery until our clock reaches the frame's
            // timestamp, and extend our next mandatory sync point. Ported
            // from `Wifi.cpp:1696-1720`: `StartRX` is *not* called here --
            // it fires later, from `Wifi::tick`'s `rx_timestamp` check,
            // once the simulated clock actually reaches this frame's
            // timestamp.
            self.rx_timestamp = timestamp_us.max(self.us_timestamp);
            self.next_sync = self.rx_timestamp + frame_time_us(cropped_framelen as usize, tx_rate);

            // Which frame this is, and therefore how far the client may run
            // before its next mandatory sync, is decided from the frame's own
            // address-1 MAC -- never from the transport's [`MpFrameKind`] tag.
            // melonDS has no out-of-band tag to consult and compares
            // `MPCmdMAC`/`MPAckMAC` here (`Wifi.cpp:1701-1714`), and
            // [`Wifi::classify_rxflags`] already derives the same distinction
            // from the same bytes for the RX header. Trusting the tag left two
            // consumers of one distinction free to disagree, so a mistagged
            // frame advanced the MP clock by the wrong amount with nothing
            // detecting the mismatch. See `docs/design/review_mp_local2.md`
            // P1-1.
            if mac_eq(&self.rx_buffer, 12 + 4, MP_CMD_MAC) {
                let client_time =
                    self.rx_buffer[12 + 24] as u16 | (self.rx_buffer[12 + 25] as u16) << 8;
                let client_mask =
                    self.rx_buffer[12 + 26] as u16 | (self.rx_buffer[12 + 27] as u16) << 8;
                let num_clients = (1..16u16).filter(|i| client_mask & (1 << i) != 0).count();
                self.next_sync += 112 + (client_time as u64 + 10) * num_clients as u64;
                // The reply/blank-reply trigger itself fires from
                // `step_rx` at completion, keyed off `rxflags` -- see
                // `docs/design/local-mp-melonds-parity.md` Gap 3.1/3.2.
            } else if mac_eq(&self.rx_buffer, 12 + 4, MP_ACK_MAC) {
                // The run-ahead window the host granted, read out of the ack
                // frame's own hardware header (`*(u32*)&RXBuffer[0]`,
                // `Wifi.cpp:1712-1714`) rather than out of transport metadata.
                // `Wifi::send_mp_ack` writes it there, so the two agree today;
                // reading the frame keeps them from ever diverging. See
                // `docs/design/review_mp_local2.md` P2-1.
                let runahead = u32::from_le_bytes([
                    self.rx_buffer[0],
                    self.rx_buffer[1],
                    self.rx_buffer[2],
                    self.rx_buffer[3],
                ]);
                self.next_sync += u64::from(runahead);
            }

            self.rx_deferred = DeferredRxParams {
                armed: true,
                keep: mac_good,
                tx_rate,
                framelen: cropped_framelen,
            };
        } else {
            self.rx_timestamp = 0;
            self.start_rx(request, mac_good, tx_rate, cropped_framelen);
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
    pub(super) fn start_rx(
        &mut self,
        request: &mut InterruptRequest,
        keep: bool,
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
        // (`Wifi.cpp:1909-1936`). This port writes the body in one step rather
        // than pumping it, so it detects the same condition up front and drops
        // the frame instead of corrupting the ring. Dropping the whole frame
        // rather than keeping the bytes that fit is a deliberate and
        // behaviourally equivalent simplification: melonDS abandons the frame
        // without calling `FinishRX`, so the partial bytes it already wrote
        // never gain a header, an IRQ 0, or an MP-reply trigger either.
        // See `docs/design/local-mp-melonds-parity-2.md` F8e.
        //
        // The check is skipped entirely until the driver has published a read
        // cursor. melonDS compares only *after* writing at least one halfword,
        // so a still-zero `W_RXBufReadCursor` cannot reject a frame there; here
        // the comparison happens up front, where a zero cursor aliases the ring
        // base and would drop the first frames of every session -- exactly the
        // frames that carry authentication and association. See
        // [`Wifi::rx_read_cursor_written`] and
        // `docs/design/review_mp_local2.md` P0-4.
        let read_cursor = ((self.ioport(W_RXBufReadCursor) as u32) << 1) & 0x1FFE;
        let body_len = cropped_framelen as usize;
        let mut probe = addr;
        let mut overruns = false;
        let mut i = 0;
        while self.rx_read_cursor_written && i < body_len {
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
            header_addr: header_addr as u16,
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

        // Reject a WEP frame while WEP processing is off (`Wifi.cpp:1277-1283`).
        let frame_ctl = self.rx_buffer[12] as u16 | (self.rx_buffer[13] as u16) << 8;
        if frame_ctl & (1 << 14) != 0 && self.ioport(W_WEPCnt) & (1 << 15) == 0 {
            self.diag.drops.wep_off += 1;
            return;
        }

        // Classification and the `W_RXFilter`/`W_RXFilter2` drop decisions
        // happen *here*, at completion, exactly where melonDS runs them
        // (`FinishRX`, `Wifi.cpp:1285-1457`) -- not at arrival in
        // [`Wifi::check_rx`]. A rejected frame gets no RX header, no cursor
        // advance, no IRQ 0 and no MP-reply/beacon trigger, but the timing
        // updates `check_rx` already applied stand, so the MP sync clock keeps
        // gating. See `docs/design/review_mp_local2.md` P0-1.
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
            return;
        };

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

        if !cmd_dupe {
            let base = self.ioport(W_RXBufBegin) as u32 & 0x1FFE;
            let end = self.ioport(W_RXBufEnd) as u32 & 0x1FFE;

            // Each field advances through `increment_rx_addr`, exactly as
            // `FinishRX` does (`Wifi.cpp:1466-1478`), so a header that
            // straddles the end of the RX ring wraps back to its start.
            //
            // Plain addition (what this used to do) writes the tail of such
            // a header past `W_RXBufEnd` while the body -- which already
            // wrapped correctly in `Wifi::start_rx` -- lands at the ring
            // base. The driver then reads a frame whose length and rate
            // fields are garbage. With the 2 KiB ring these games use, a
            // header straddles the wrap every couple of dozen frames, so a
            // steady fraction of traffic is corrupted indefinitely.
            let mut addr = pending.header_addr as u32;
            let write_field = |ram: &mut [u8], addr: &mut u32, step: u32, value: u16| {
                let off = *addr as usize;
                if off + 1 < ram.len() {
                    ram[off] = value as u8;
                    ram[off + 1] = (value >> 8) as u8;
                }
                increment_rx_addr(addr, step, base, end);
            };
            write_field(&mut self.ram, &mut addr, 2, rxflags);
            write_field(&mut self.ram, &mut addr, 4, 0x0040);
            write_field(&mut self.ram, &mut addr, 2, u16::from(pending.tx_rate));
            write_field(&mut self.ram, &mut addr, 2, pending.framelen);
            write_field(&mut self.ram, &mut addr, 2, 0x4080);

            let mut cursor = u32::from(self.ioport(W_RXTXAddr)) << 1;
            if cursor & 0x2 != 0 {
                increment_rx_addr(&mut cursor, 2, base, end);
            }
            self.set_ioport(W_RXBufWriteCursor, ((cursor & !0x3) >> 1) as u16);

            self.raise_irq(0, request);

            // An association response just became visible to the driver. Dump
            // exactly what was committed, then arm the read trace so the next
            // `W_RXBufDataRead` reads can be compared against it. See
            // [`super::assoc_trace_enabled`].
            if super::assoc_trace_enabled() && frame_ctl & 0x00FF == 0x0010 {
                let hdr = pending.header_addr as usize;
                let peek = |off: usize| -> u16 {
                    let a = (hdr + off) & 0x1FFE;
                    u16::from(self.ram[a]) | u16::from(self.ram[a + 1]) << 8
                };
                // Both instances share one stderr, so tag every line with the
                // local MAC's high word -- the one value guaranteed to differ
                // between them (see `loader::load_rom_for_instance`).
                let who = self.ioport(W_MACAddr2);
                eprintln!(
                    "[assoc-trace][{who:04X}] committed assoc-resp: header@0x{hdr:04X} \
                     rxflags=0x{:04X} rate=0x{:04X} framelen={} write_cursor=0x{:04X} \
                     read_cursor=0x{:04X} ring=0x{:04X}..0x{:04X}",
                    peek(0),
                    peek(6),
                    peek(8),
                    self.ioport(W_RXBufWriteCursor),
                    self.ioport(W_RXBufReadCursor),
                    self.ioport(W_RXBufBegin),
                    self.ioport(W_RXBufEnd),
                );
                let body: Vec<String> =
                    (0..16).map(|i| format!("{:02X}", self.rx_buffer[12 + i])).collect();
                let field = |off: usize| -> u16 {
                    u16::from(self.rx_buffer[12 + 24 + off])
                        | u16::from(self.rx_buffer[12 + 24 + off + 1]) << 8
                };
                // The association-response body is capability / status / AID.
                // **Status is what decides acceptance**: a nonzero code is a
                // refusal the host itself sent, which would mean the fault is
                // on the host's side of the handshake, not in this RX path.
                eprintln!(
                    "[assoc-trace][{who:04X}]   body[0..16] = {} (capability=0x{:04X} \
                     status=0x{:04X} aid=0x{:04X})",
                    body.join(" "),
                    field(0),
                    field(2),
                    field(4),
                );
                self.assoc_trace_reads = Wifi::ASSOC_TRACE_READS;
            }
        }

        match rxflags & 0x800F {
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
        // No `W_PowerState` bit-9 gate here either; see `Wifi::check_rx`.
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

        // Same `W_RXLenCrop` handling as `Wifi::crop_framelen`, including
        // the WEP body slide (`Wifi.cpp:1544-1552`).
        let reply_ctl = reply[12] as u16 | (reply[13] as u16) << 8;
        let crop = self.ioport(W_RXLenCrop);
        framelen = if reply_ctl & (1 << 14) != 0 {
            framelen.saturating_sub((crop >> 7) & 0x1FE)
        } else {
            framelen.saturating_sub((crop << 1) & 0x1FE)
        };

        let total = (12 + framelen as usize).min(reply.len()).min(self.rx_buffer.len());
        self.rx_buffer[..total].copy_from_slice(&reply[..total]);

        // No classification here either: like the regular path, this frame is
        // classified and filtered at completion in [`Wifi::step_rx`], matching
        // melonDS's `MPClientReplyRX`, which likewise only calls `StartRX` and
        // leaves `FinishRX` to do the rest (`Wifi.cpp:1554-1561`).
        let mac_good = self.rx_buffer[16] & 0x01 != 0 || self.mac_matches(&self.rx_buffer[16..22]);

        self.rx_timestamp = 0;
        self.start_rx(request, mac_good, tx_rate, framelen);
    }
}

fn frame_time_us(frame_len: usize, tx_rate: u8) -> u64 {
    frame_len as u64 * if tx_rate == 0x14 { 4 } else { 8 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::net::wifi::mp::{LoopbackTransport, MpTransport};

    /// Builds a well-formed inbound frame: the 12-byte hardware header
    /// (rate, channel and length filled in) followed by an 802.11 frame whose
    /// frame-control field is `frame_ctl`. `body_len` counts the 802.11 frame,
    /// which must be at least the 24-byte header `check_rx` requires.
    fn frame(channel: u8, frame_ctl: u16, body_len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; 12 + body_len];
        buf[8] = 0x14; // 2 Mbit/s.
        buf[9] = channel;
        buf[10] = body_len as u8;
        buf[11] = (body_len >> 8) as u8;
        buf[12] = frame_ctl as u8;
        buf[13] = (frame_ctl >> 8) as u8;
        buf
    }

    /// A `Wifi` with reception armed, a configured RX ring and a resolved
    /// channel -- everything `check_rx` gates on before it looks at a frame --
    /// paired with the loopback transport its peer sends on.
    fn armed_wifi() -> (Wifi, LoopbackTransport) {
        let (host, client) = LoopbackTransport::new_pair();
        let mut wifi = Wifi::new();
        wifi.set_transport(Some(Box::new(client)));
        wifi.cur_channel = 6;
        wifi.set_ioport(W_RXCnt, 0x8000);
        wifi.set_ioport(W_RXBufBegin, 0x4000);
        wifi.set_ioport(W_RXBufEnd, 0x4800);
        (wifi, host)
    }

    /// melonDS skips MP command and reply frames on the regular RX path while
    /// `IsMP` is false (`Wifi.cpp:1620-1628`). Without that, a non-associated
    /// instance writes another session's CMD frames into its RX ring and --
    /// through `Wifi::step_rx`'s `0x800C` branch -- transmits blank replies
    /// into an exchange it is not part of. `docs/design/review_mp_local2.md`
    /// P0-5.
    #[test]
    fn mp_frames_are_ignored_while_not_in_an_mp_session() {
        let (mut wifi, mut peer) = armed_wifi();
        let mut cmd = frame(6, 0x0008, 32);
        cmd[12 + 4..12 + 10].copy_from_slice(&MP_CMD_MAC);
        peer.send_packet(&cmd, 1_000);

        let mut request = InterruptRequest::empty();
        assert!(!wifi.check_rx(RxKind::Regular, &mut request), "the CMD frame must be skipped");
        assert_eq!(wifi.diag.drops.foreign_mp, 1);
        assert_eq!(wifi.diag.rx_accepted, 0, "it must never reach the RX pump");
    }

    /// `CheckRX` is a `for (;;)` loop: an invalid frame is skipped and the next
    /// one is pulled within the same call (`Wifi.cpp:1581-1645`). Returning on
    /// the first rejection cost a whole polling opportunity per bad frame,
    /// which during association -- where channel mismatches arrive in runs --
    /// stalls the handshake. `docs/design/review_mp_local2.md` P0-2.
    #[test]
    fn check_rx_drains_past_invalid_frames_within_one_call() {
        let (mut wifi, mut peer) = armed_wifi();
        // Three rejects, one per validation rule, then a good frame.
        peer.send_packet(&frame(6, 0x0008, 8), 1_000); // Too short.
        let mut bad_len = frame(6, 0x0008, 32);
        bad_len[10] = 99; // Length field disagrees with the datagram.
        peer.send_packet(&bad_len, 1_000);
        peer.send_packet(&frame(11, 0x0008, 32), 1_000); // Wrong channel.
        peer.send_packet(&frame(6, 0x0008, 32), 1_000);

        let mut request = InterruptRequest::empty();
        assert!(wifi.check_rx(RxKind::Regular, &mut request), "the valid frame must be accepted");
        assert_eq!(wifi.diag.rx_accepted, 1);
        assert_eq!(wifi.diag.drops.too_short, 1);
        assert_eq!(wifi.diag.drops.bad_length, 1);
        assert_eq!(wifi.diag.drops.channel_mismatch, 1);
    }

    /// Before the driver publishes a read cursor, `W_RXBufReadCursor` reads
    /// zero, which after the halfword shift aliases the ring base. Testing the
    /// overrun condition against it up front therefore rejected the opening
    /// frames of every session -- exactly the authentication and association
    /// traffic a link needs. `docs/design/review_mp_local2.md` P0-4.
    #[test]
    fn ring_overrun_check_waits_for_the_driver_to_publish_a_read_cursor() {
        let (mut wifi, mut peer) = armed_wifi();
        assert!(!wifi.rx_read_cursor_written, "precondition: the driver has not written it");
        peer.send_packet(&frame(6, 0x0008, 32), 1_000);

        let mut request = InterruptRequest::empty();
        assert!(wifi.check_rx(RxKind::Regular, &mut request));
        assert_eq!(wifi.diag.drops.ring_full, 0, "the frame must not be dropped as an overrun");
        assert_ne!(wifi.com_status & 0x1, 0, "the RX pump must have started");
    }

    /// melonDS advances `NextSync` in `CheckRX`, before `FinishRX` ever applies
    /// `W_RXFilter`. Filtering at arrival instead meant a filtered frame never
    /// reached the timing update, so an MP client's clock stopped gating and
    /// `Wifi::tick` re-entered `check_rx` every 8µs tick.
    /// `docs/design/review_mp_local2.md` P0-1.
    #[test]
    fn a_filtered_frame_still_advances_the_mp_sync_clock() {
        let (mut wifi, mut peer) = armed_wifi();
        wifi.is_mp = true;
        wifi.is_mp_client = true;
        // Address 1 must match us for the client-pacing branch to be taken.
        wifi.set_ioport(W_MACAddr0, 0x0201);
        wifi.set_ioport(W_MACAddr1, 0x0403);
        wifi.set_ioport(W_MACAddr2, 0x0605);
        let mut f = frame(6, 0x0008, 32);
        f[12 + 4..12 + 10].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        peer.send_packet(&f, 50_000);

        let mut request = InterruptRequest::empty();
        assert!(wifi.check_rx(RxKind::HostFrames, &mut request));
        assert!(wifi.next_sync >= 50_000, "next_sync must be derived from the frame's timestamp");

        // The frame is filtered at completion: `W_BSSID` is zero so it belongs
        // to no network of ours, and `W_RXFilter` bit 11 is clear.
        wifi.rx_timestamp = 0;
        let d = wifi.rx_deferred;
        wifi.start_rx(&mut request, d.keep, d.tx_rate, d.framelen);
        while wifi.com_status & 0x1 != 0 {
            wifi.step_rx(&mut request);
        }
        assert_eq!(
            wifi.diag.drops.filtered, 1,
            "the frame is dropped by the filter, at completion"
        );
    }
}
