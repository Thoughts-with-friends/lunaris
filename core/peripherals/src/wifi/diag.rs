//! Counters of how far the Wi-Fi handshake gets, for multiplayer debugging.
//!
//! ```text
//!  stage            counters
//!  1 radio          rf_transfers, channel, channel_changes
//!  2 driver setup   mode_reset, rxbuf_cfg
//!  3 transmitting   loc_tx, beacon_tx, cmd_tx, reply_tx, blank_reply_tx,
//!                   ack_tx, tx_dropped_no_channel
//!  4 receiving      rx_polls, rx_empty, rx_accepted, drops
//!  5 classified     rxflags_* (what the RX filter made of accepted frames)
//!  6 associated     last_assoc_*, aid, is_mp / is_mp_client
//!  7 delivered      irq0_raised, irq0_masked, rx_ring_reads
//! ```
//!
//! [`MpDiag::verdict`] names the earliest stage that did not happen. The
//! counters are not part of savestates.

/// Frames the receiver threw away, by reason.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RxDrops {
    /// The receiver was off (W_RXCNT bit 15 clear or powered down).
    pub rx_disabled: u32,
    /// Shorter than a TX header plus an 802.11 header.
    pub too_short: u32,
    /// The TX header's length disagrees with the frame.
    pub bad_length: u32,
    /// Sent on another channel (or this console is not tuned).
    pub channel_mismatch: u32,
    /// MP traffic while this console is not in a MP session.
    pub foreign_mp: u32,
    /// Addressed to another console, or WEP while WEP is off.
    pub not_for_us: u32,
    /// Rejected by W_RXFILTER / W_RXFILTER2.
    pub filtered: u32,
}

/// A snapshot of the counters (see the module docs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MpDiag {
    pub mac: [u8; 6],
    pub link: bool,
    pub powered: bool,

    pub rf_version: u8,
    pub rf_transfers: u32,
    pub channel: u8,
    pub channel_changes: u32,

    pub mode_reset: u32,
    pub rxbuf_cfg: u32,

    pub loc_tx: u32,
    pub beacon_tx: u32,
    pub cmd_tx: u32,
    pub reply_tx: u32,
    pub blank_reply_tx: u32,
    pub ack_tx: u32,
    pub tx_dropped_no_channel: u32,

    pub rx_polls: u32,
    pub rx_empty: u32,
    pub rx_accepted: u32,
    pub drops: RxDrops,

    pub rxflags_beacon: u32,
    pub rxflags_mgmt: u32,
    pub rxflags_data: u32,
    pub rxflags_cmd: u32,
    pub rxflags_ack: u32,
    pub rxflags_reply: u32,

    pub last_assoc_aid: u16,
    pub last_assoc_mac_good: bool,
    pub last_assoc_timestamp: u64,
    pub aid: u16,
    pub is_mp: bool,
    pub is_mp_client: bool,

    pub irq0_raised: u32,
    pub irq0_masked: u32,
    pub rx_ring_reads: u32,
}

impl MpDiag {
    /// Counts an accepted frame by its RX header frame type (bits 0-3).
    pub(super) const fn classify(&mut self, rxflags: u16) {
        // DS Wifi Hardware Headers: https://problemkaputt.de/gbatek.htm#dswifihardwareheaders
        match rxflags & 0xF {
            0x1 => self.rxflags_beacon += 1,
            0x0 | 0x5 => self.rxflags_mgmt += 1,
            0xC => self.rxflags_cmd += 1,
            0xD | 0xE => self.rxflags_ack += 1,
            0xF => self.rxflags_reply += 1,
            _ => self.rxflags_data += 1,
        }
    }

    /// The earliest stage that has not happened, as one sentence.
    pub fn verdict(&self) -> String {
        let d = &self.drops;
        if !self.link {
            return "VERDICT: no MP link installed -- this console is not in a session.".to_owned();
        }
        if !self.powered {
            return "VERDICT: the game has not powered the Wi-Fi controller on yet.".to_owned();
        }
        if self.rf_transfers == 0 {
            return "VERDICT: the driver never programmed the RF chip, so no channel was \
                    selected -- the game's Wi-Fi init did not reach the radio."
                .to_owned();
        }
        if self.channel == 0 {
            return format!(
                "VERDICT: the RF registers match no channel of the firmware's RF table \
                 (RF chip type {}), so nothing can be sent or received.",
                self.rf_version
            );
        }
        if self.mode_reset == 0 && self.rxbuf_cfg == 0 {
            return "VERDICT: the driver never configured the RX ring (no W_MODE_RST and no \
                    W_RXBUF_BEGIN/END writes) -- reception cannot start."
                .to_owned();
        }
        let sent = self.loc_tx + self.beacon_tx + self.cmd_tx + self.reply_tx + self.blank_reply_tx;
        if sent == 0 {
            return format!(
                "VERDICT: nothing was transmitted ({} frames dropped while untuned) -- the \
                 driver never armed a TX slot.",
                self.tx_dropped_no_channel
            );
        }
        if self.rx_accepted == 0 {
            return format!(
                "VERDICT: frames are sent but none was accepted ({} polls, {} empty; dropped: \
                 off {} short {} length {} channel {} foreign-MP {} not-for-us {} filtered {}) \
                 -- check the peer's channel and MAC.",
                self.rx_polls,
                self.rx_empty,
                d.rx_disabled,
                d.too_short,
                d.bad_length,
                d.channel_mismatch,
                d.foreign_mp,
                d.not_for_us,
                d.filtered
            );
        }
        if self.irq0_raised == 0 {
            return "VERDICT: frames reach the RX ring but no RX-complete IRQ was raised."
                .to_owned();
        }
        if !self.is_mp {
            return if self.last_assoc_timestamp == 0 {
                "VERDICT: frames are exchanged but no association response arrived yet -- \
                 the consoles have not paired."
                    .to_owned()
            } else {
                format!(
                    "VERDICT: an association response arrived (aid {}, addressed to us: {}) but \
                     no MP session started.",
                    self.last_assoc_aid, self.last_assoc_mac_good
                )
            };
        }
        format!(
            "VERDICT: in a MP session as {} (aid {}).",
            if self.is_mp_client { "client" } else { "host" },
            self.aid
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_names_the_earliest_missing_stage() {
        let mut d = MpDiag::default();
        assert!(d.verdict().contains("no MP link"));
        d.link = true;
        assert!(d.verdict().contains("not powered"));
        d.powered = true;
        assert!(d.verdict().contains("never programmed the RF chip"));
        d.rf_transfers = 4;
        assert!(d.verdict().contains("match no channel"));
        d.channel = 7;
        assert!(d.verdict().contains("RX ring"));
        d.mode_reset = 1;
        assert!(d.verdict().contains("nothing was transmitted"));
        d.beacon_tx = 3;
        assert!(d.verdict().contains("none was accepted"));
        d.rx_accepted = 2;
        assert!(d.verdict().contains("IRQ"));
        d.irq0_raised = 2;
        assert!(d.verdict().contains("no association response"));
        d.is_mp = true;
        assert!(d.verdict().contains("host"));
    }

    #[test]
    fn classifies_rx_frame_types() {
        let mut d = MpDiag::default();
        for flags in [0x8001, 0x0010, 0x800C, 0x800D, 0x800F, 0x8008] {
            d.classify(flags);
        }
        assert_eq!(
            (d.rxflags_beacon, d.rxflags_mgmt, d.rxflags_cmd, d.rxflags_ack, d.rxflags_reply),
            (1, 1, 1, 1, 1)
        );
        assert_eq!(d.rxflags_data, 1);
    }
}
