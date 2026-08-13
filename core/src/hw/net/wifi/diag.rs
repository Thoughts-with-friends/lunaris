//! Low-volume local-multiplayer diagnostics.
//!
//! `LUNARIS_WIFI_DEBUG=1` traces every register access, which is far too
//! noisy to read (and slow enough to change timing) when the question is
//! simply *"how far did the MP handshake get?"*. This module instead keeps a
//! handful of counters on the hot path and prints one compact block every
//! [`DUMP_INTERVAL_MS`], so a session that fails to connect produces a few
//! dozen readable lines instead of megabytes.
//!
//! Enable with `LUNARIS_MP_DIAG=1`. Independent of `LUNARIS_WIFI_DEBUG`;
//! leaving it on costs a few integer increments per frame.
//!
//! The counters are ordered to mirror the MP handshake, so the first line
//! that stays at zero names the stage that is broken:
//!
//! 1. `mode_reset` / `rxbuf_cfg` — did the driver configure the hardware?
//! 2. `beacon_tx` / `cmd_tx` — is anything being transmitted?
//! 3. `rx_accepted` vs the `drop_*` counters — is anything being received,
//!    and if not, which check rejected it?
//! 4. `rxflags_*` — were the received frames classified as the MP frame
//!    types the game is waiting for?
//! 5. `replies_answered` / `irq12` — did a full CMD round complete?
//!
//! See `docs/design/local-mp-melonds-parity-2.md` §1, which this replaces
//! with something the emulator reports about itself.

/// How often (in emulated microseconds) a summary block is printed.
const DUMP_INTERVAL_US: u64 = 2_000_000;

/// Whether MP diagnostics are enabled, cached from the environment. Checked
/// on paths that run thousands of times per frame, so it must not re-read
/// the environment per call.
pub(super) fn diag_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("LUNARIS_MP_DIAG").is_some())
}

/// Reason an inbound frame was rejected, one counter per check in
/// [`super::Wifi::check_rx`]. Ordered as the checks run.
#[derive(Clone, Copy, Default)]
pub struct RxDrops {
    /// `W_RXCnt` bit 15 clear: the driver has not armed reception.
    pub rx_disabled: u32,
    /// `W_RXBufBegin == W_RXBufEnd`: the RX ring was never configured.
    pub ring_unconfigured: u32,
    /// Frame shorter than a 12-byte hardware header plus an 802.11 header.
    pub too_short: u32,
    /// The header's length field disagreed with the datagram's real length.
    pub bad_length: u32,
    /// The frame was transmitted on a different RF channel than we resolved.
    pub channel_mismatch: u32,
    /// `W_RXFilter`/`W_RXFilter2` rejected the frame.
    pub filtered: u32,
    /// The RX ring was full: the driver has not drained the previous frame.
    pub ring_full: u32,
}

/// Counters describing how far the MP handshake progressed.
#[derive(Clone, Copy, Default)]
pub struct MpDiag {
    /// Emulated microseconds since the last summary dump.
    us_since_dump: u64,

    /// `W_ModeReset` writes with bit 14 set (installs the RX/filter defaults).
    pub mode_reset: u32,
    /// Writes to `W_RXBufBegin`/`W_RXBufEnd`.
    pub rxbuf_cfg: u32,
    /// Resolved RF channel, or `0` if channel detection never succeeded.
    pub channel: i32,
    /// `RFChipType` from the firmware Wi-Fi config block (2 or 3).
    pub rf_version: u8,
    /// The two RF register ids the channel table is indexed by.
    pub rf_channel_index: [u32; 2],
    /// `true` if the parsed channel table is entirely zero -- channel
    /// detection cannot work, and either the firmware image or the parse
    /// offsets are wrong. See [`super::Wifi::load_firmware_config`].
    pub rf_table_empty: bool,
    /// Current contents of the two channel-index RF registers, i.e. what the
    /// game last asked for. Compared against the table by
    /// [`super::Wifi::change_channel`].
    pub rf_regs_now: [u32; 2],
    /// RF register transfers the driver has triggered (`W_RFData2` writes).
    /// Zero means the driver never programmed the radio at all, so no
    /// channel can ever be selected regardless of the calibration table.
    pub rf_transfers: u32,
    /// Register id and command of the most recent RF transfer, for telling
    /// "the driver wrote a different register" apart from "the driver never
    /// wrote anything".
    pub rf_last_id: u8,
    pub rf_last_cmd: u8,
    /// Baseband register writes (`W_BBWrite` with `W_BBCnt` in write mode).
    /// The driver uploads the BB table before touching the RF chip, so a
    /// zero here places the failure even earlier.
    pub bb_writes: u32,

    /// General-purpose (LOC1-3) slot transmissions -- authentication,
    /// association and every other management/data frame a client sends
    /// while joining. Counted separately because these, not beacons or MP
    /// frames, are what a *guest* transmits before it associates.
    pub loc_tx: u32,
    /// Beacon slot transmissions.
    pub beacon_tx: u32,
    /// MP CMD slot transmissions.
    pub cmd_tx: u32,
    /// MP reply transmissions (staged frame, not the blank keep-alive).
    pub reply_tx: u32,
    /// Blank keep-alive replies sent because we had nothing staged.
    pub blank_reply_tx: u32,

    /// Times the hardware actually asked the transport for a frame. Lets
    /// "we polled and the peer sent nothing" be told apart from "we never
    /// polled", which the drop counters alone cannot express.
    pub rx_polls: u32,
    /// Polls that came back empty.
    pub rx_empty: u32,
    /// Frames that passed every check and entered the RX pump.
    pub rx_accepted: u32,
    /// Per-reason rejection counters.
    pub drops: RxDrops,

    /// Accepted beacons whose BSSID matched ours (`rxflags & 0x800F == 0x8001`).
    pub rxflags_beacon: u32,
    /// Accepted MP CMD frames addressed to our BSSID (`0x800C`).
    pub rxflags_cmd: u32,
    /// Accepted MP reply frames (`0x800E`/`0x800F`).
    pub rxflags_reply: u32,
    /// Accepted MP ack frames (`0x800D`).
    pub rxflags_ack: u32,
    /// Accepted management frames (association/authentication traffic).
    pub rxflags_mgmt: u32,
    /// Accepted management frames broken down by 802.11 subtype, indexed by
    /// `(frame_control >> 4) & 0xF`. Lumping them together hides the thing
    /// that actually matters while a link is forming: whether an
    /// association *response* ever arrives, and whether a deauthentication
    /// is what tears the session down.
    pub rx_mgmt_subtype: [u32; 16],
    /// The most recent received authentication frame's body fields:
    /// `[algorithm, sequence, status]` from body offsets 0/2/4. A stalled
    /// 802.11 authentication is invisible from frame counts alone -- these
    /// three values say whether the responder ever answers with sequence 2
    /// and status 0 (success), or keeps repeating sequence 1.
    pub last_auth: [u16; 3],
    /// How many received frames carried the 802.11 retransmit flag
    /// (frame-control bit 11). A driver discards what it believes are
    /// duplicates, so every frame arriving flagged as a retry stalls a
    /// handshake while the frame counters still look healthy.
    pub rx_retry_flagged: u32,

    /// Non-zero `answered` masks returned by the transport's reply collection.
    pub replies_answered: u32,
    /// Reply collections that returned an empty mask.
    pub replies_empty: u32,
    /// IRQ 12 (MP CMD transaction complete) raises.
    pub irq12: u32,

    /// `true` once this instance believes it is in an MP session.
    pub is_mp: bool,
    /// `true` once this instance associated as a client (has an AID).
    pub is_mp_client: bool,
    /// Association id granted by the host, or `0`.
    pub aid: u16,
    /// This instance's programmed MAC address. Two instances that share one
    /// can never complete 802.11 authentication with each other, so it is
    /// shown rather than left to be inferred from an endless `auth`
    /// exchange.
    pub mac: [u8; 6],
    /// The five most-read Wi-Fi registers as `(port, count)`, descending.
    /// A driver stuck in a readiness poll spins on one port, so this names
    /// what it is waiting for. Filled in by [`super::Wifi::diag_snapshot`].
    pub top_reads: [(u16, u32); 5],
    /// Whether a frontend-supplied MP transport is currently installed --
    /// i.e. whether this instance is in a room at all. Refreshed by
    /// [`super::Wifi::diag_snapshot`]; not maintained by the counters.
    pub transport_installed: bool,
}

impl MpDiag {
    /// Advances the dump timer, printing a summary block every
    /// [`DUMP_INTERVAL_US`] of emulated time.
    ///
    /// Driven from [`super::Wifi::tick`], which runs unconditionally, rather
    /// than from `ms_timer`: the millisecond timer only advances while the
    /// driver has enabled `W_USCountCnt`, and "the driver never enabled the
    /// microsecond counter" is itself one of the failures this summary needs
    /// to be able to report.
    pub(super) fn tick_us(&mut self, interval_us: u64, transport_installed: bool) {
        if !diag_enabled() {
            return;
        }
        self.us_since_dump += interval_us;
        if self.us_since_dump < DUMP_INTERVAL_US {
            return;
        }
        self.us_since_dump = 0;
        self.dump(transport_installed);
    }

    /// Forces a summary block regardless of the dump timer. Used by tests
    /// and by any caller that wants a snapshot at a specific moment.
    pub fn dump_now(&self, transport_installed: bool) {
        self.dump(transport_installed);
    }

    /// Prints one summary block. Read it top-down: the first stage whose
    /// counters are all zero is where the handshake stopped.
    fn dump(&self, transport_installed: bool) {
        let d = &self.drops;
        eprintln!(
            "\n[mp-diag] ---- local multiplayer status ----\n\
             [mp-diag] transport_installed={transport_installed} channel={} is_mp={} is_mp_client={} aid={}\n\
             [mp-diag] rf: chip_type={} index=[{},{}] regs_now=[0x{:X},0x{:X}] table_empty={}\n\
             [mp-diag] rf: transfers={} last_id={} last_cmd={} bb_writes={}\n\
             [mp-diag] 1. driver setup   : mode_reset={} rxbuf_cfg={}\n\
             [mp-diag] 2. transmitted    : loc={} beacon={} cmd={} reply={} blank_reply={}\n\
             [mp-diag] 3. received       : accepted={} polls={} empty={}\n\
             [mp-diag]    dropped        : rx_disabled={} ring_unconfigured={} too_short={} bad_length={}\n\
             [mp-diag]                     channel_mismatch={} filtered={} ring_full={}\n\
             [mp-diag] 4. classified     : beacon={} cmd={} reply={} ack={} mgmt={}\n\
             [mp-diag] 5. round complete : replies_answered={} replies_empty={} irq12={}
             [mp-diag] most-read regs  : {}",
            self.channel,
            self.is_mp,
            self.is_mp_client,
            self.aid,
            self.rf_version,
            self.rf_channel_index[0],
            self.rf_channel_index[1],
            self.rf_regs_now[0],
            self.rf_regs_now[1],
            self.rf_table_empty,
            self.rf_transfers,
            self.rf_last_id,
            self.rf_last_cmd,
            self.bb_writes,
            self.mode_reset,
            self.rxbuf_cfg,
            self.loc_tx,
            self.beacon_tx,
            self.cmd_tx,
            self.reply_tx,
            self.blank_reply_tx,
            self.rx_accepted,
            self.rx_polls,
            self.rx_empty,
            d.rx_disabled,
            d.ring_unconfigured,
            d.too_short,
            d.bad_length,
            d.channel_mismatch,
            d.filtered,
            d.ring_full,
            self.rxflags_beacon,
            self.rxflags_cmd,
            self.rxflags_reply,
            self.rxflags_ack,
            self.rxflags_mgmt,
            self.replies_answered,
            self.replies_empty,
            self.irq12,
            self
                .top_reads
                .iter()
                .filter(|&&(_, n)| n > 0)
                .map(|&(reg, n)| format!("{reg:03X}:{n}"))
                .collect::<Vec<_>>()
                .join("  "),
        );
        eprintln!("[mp-diag] {}", self.verdict(transport_installed));
    }

    /// A one-line reading of the counters, naming the earliest stage that
    /// looks broken. Heuristic, but it is the same reasoning
    /// `docs/design/local-mp-melonds-parity-2.md` §1 asks a human to apply.
    pub fn verdict(&self, transport_installed: bool) -> String {
        let d = &self.drops;
        if !transport_installed {
            return "VERDICT: no MP transport installed -- this instance is not in a room."
                .to_string();
        }
        if self.channel == 0 {
            if self.rf_transfers == 0 {
                return format!(
                    "VERDICT: the driver never programmed the RF chip (0 RF transfers, {} BB                      writes), so no channel was ever selected. The failure is upstream of the                      channel table -- the game's Wi-Fi init did not get as far as the radio.",
                    self.bb_writes
                );
            }
            if self.rf_table_empty {
                return format!(
                    "VERDICT: RF channel never resolved -- the firmware's channel table is all \
                     zeros (RFChipType={}). The firmware image is not a usable dump, or it was \
                     parsed at the wrong offsets.",
                    self.rf_version
                );
            }
            return format!(
                "VERDICT: RF channel never resolved. The game asked for RF[{}]=0x{:X} \
                 RF[{}]=0x{:X}, which matches no entry in the firmware's RFChipType={} channel \
                 table, so nothing can be sent or received.",
                self.rf_channel_index[0],
                self.rf_regs_now[0],
                self.rf_channel_index[1],
                self.rf_regs_now[1],
                self.rf_version,
            );
        }
        if self.mode_reset == 0 && self.rxbuf_cfg == 0 {
            return "VERDICT: the driver never configured the RX ring (no W_ModeReset bit14 and \
                    no W_RXBufBegin/End writes) -- reception cannot start."
                .to_string();
        }
        if self.beacon_tx == 0 && self.cmd_tx == 0 && self.reply_tx == 0 && self.blank_reply_tx == 0
        {
            return "VERDICT: nothing has been transmitted. The game's driver has not armed any \
                    TX slot; look upstream of the Wi-Fi hardware."
                .to_string();
        }
        if self.rx_accepted == 0 {
            let worst = [
                (d.rx_disabled, "W_RXCnt bit15 never set (reception not armed)"),
                (d.ring_unconfigured, "RX ring unconfigured (W_RXBufBegin == W_RXBufEnd)"),
                (d.channel_mismatch, "channel mismatch between the two instances"),
                (d.filtered, "W_RXFilter/W_RXFilter2 rejected every frame"),
                (d.bad_length, "frame length field disagreed with the datagram"),
                (d.too_short, "frames too short to be valid"),
                (d.ring_full, "RX ring full (driver never drained it)"),
            ]
            .into_iter()
            .max_by_key(|&(n, _)| n);
            // A handful of drops during boot is normal; only call a drop
            // reason the cause when it actually dominates the traffic.
            return match worst {
                Some((n, why)) if n > 0 && n * 4 >= self.rx_polls => {
                    format!("VERDICT: no frame was ever accepted. Dominant reason: {why}.")
                }
                _ if self.rx_polls > 0 && self.rx_empty == self.rx_polls => format!(
                    "VERDICT: reception is armed and polling ({} times), but the peer has sent                      nothing at all. The problem is on the *other* instance's transmit side, or                      in the room/UDP layer between them.",
                    self.rx_polls
                ),
                _ => "VERDICT: no frame ever reached this instance at all -- the transport is \
                      delivering nothing. Check the room/UDP layer, not the Wi-Fi hardware."
                    .to_string(),
            };
        }
        if self.rxflags_beacon == 0 && self.rxflags_mgmt == 0 && !self.is_mp {
            return "VERDICT: frames are arriving but none classified as a beacon or management \
                    frame for our BSSID, so association never starts."
                .to_string();
        }
        if self.cmd_tx > 0 && self.replies_answered == 0 {
            return "VERDICT: the host is sending CMD frames but no client reply is being \
                    collected -- the round always times out."
                .to_string();
        }
        if self.rxflags_cmd > 0 && self.reply_tx == 0 && self.blank_reply_tx > 0 {
            return "VERDICT: this client receives CMD frames but only ever sends blank replies \
                    -- its AID is not in the host's clientmask, or no reply frame is staged."
                .to_string();
        }
        if self.irq12 == 0 && self.cmd_tx > 0 {
            return "VERDICT: CMD rounds start but never complete (no IRQ 12).".to_string();
        }
        "VERDICT: the MP handshake looks healthy at the hardware layer.".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The verdict must name the *earliest* broken stage, so a session that
    /// fails early is not misdiagnosed by a later counter that is trivially
    /// zero as a consequence.
    #[test]
    fn verdict_names_the_earliest_broken_stage() {
        let mut d = MpDiag::default();

        // No transport at all.
        assert!(d.verdict(false).contains("no MP transport"));

        // Transport present, but the driver never programmed the radio at all.
        assert!(d.verdict(true).contains("never programmed the RF chip"));

        // Radio programmed, but the values match no channel table entry.
        d.rf_transfers = 4;
        assert!(d.verdict(true).contains("RF channel never resolved"));

        // An all-zero table is reported as a firmware problem, not a mismatch.
        d.rf_table_empty = true;
        assert!(d.verdict(true).contains("channel table is all zeros"));
        d.rf_table_empty = false;

        // Channel fine, but the driver never configured the RX ring.
        d.channel = 7;
        assert!(d.verdict(true).contains("never configured the RX ring"));

        // Ring configured, but nothing has been transmitted.
        d.mode_reset = 1;
        assert!(d.verdict(true).contains("nothing has been transmitted"));

        // Transmitting, but every inbound frame is filtered away.
        d.beacon_tx = 10;
        d.drops.filtered = 40;
        d.drops.channel_mismatch = 2;
        let v = d.verdict(true);
        assert!(v.contains("no frame was ever accepted"), "{v}");
        assert!(v.contains("W_RXFilter"), "dominant reason must be the largest counter: {v}");

        // Frames arriving and classified, host sending CMDs, but no replies.
        d.drops = RxDrops::default();
        d.rx_accepted = 50;
        d.rxflags_beacon = 5;
        d.cmd_tx = 20;
        assert!(d.verdict(true).contains("no client reply"));

        // Replies collected and rounds completing.
        d.replies_answered = 20;
        d.irq12 = 20;
        assert!(d.verdict(true).contains("healthy"));
    }

    /// A frame that arrived but was never accepted, with *no* drop counter
    /// set, means the transport delivered nothing at all -- a different
    /// problem from a hardware-layer rejection, and the verdict must say so.
    #[test]
    fn verdict_distinguishes_silent_transport_from_a_rejection() {
        let d = MpDiag {
            channel: 7,
            rf_transfers: 4,
            mode_reset: 1,
            beacon_tx: 5,
            ..MpDiag::default()
        };
        let v = d.verdict(true);
        assert!(v.contains("transport is delivering nothing"), "{v}");
    }
}
