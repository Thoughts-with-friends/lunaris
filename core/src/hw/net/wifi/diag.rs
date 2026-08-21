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
//! # Reading stages 3 and 4 after the deferred-filtering change
//! Frame acceptance is split across the simulated transfer time, so these two
//! stages no longer partition the traffic between them (see
//! `docs/design/review_mp_local2.md` P0-1):
//!
//! * `rx_accepted` counts frames that cleared [`super::Wifi::check_rx`]. The
//!   `W_RXFilter`/`W_RXFilter2` decision happens later, in
//!   [`super::Wifi::step_rx`], so **`rx_accepted` and `drops.filtered` can both
//!   be high for the same frames**. A large `filtered` next to a healthy
//!   `rx_accepted` means "the radio hears the peer, but the driver's filters are
//!   rejecting it" — a very different fault from a low `rx_accepted`.
//! * The `rxflags_*` counters are likewise bumped in `step_rx`, which
//!   [`super::Wifi::mp_client_reply_rx`] also flows through. On a **host**,
//!   `rxflags_reply` therefore includes the client replies the host re-injects
//!   into its own RX path, not only replies observed on the wire.
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

/// Reason an inbound frame was rejected, one counter per rejecting check.
///
/// Ordered as the checks run, which spans three functions since frame
/// acceptance is split across the simulated transfer time (see
/// `docs/design/review_mp_local2.md` P0-1):
///
/// * [`super::Wifi::check_rx`] — `rx_disabled` … `foreign_mp`, at arrival;
/// * [`super::Wifi::start_rx`] — `ring_full`, when the body is written;
/// * [`super::Wifi::step_rx`] — `wep_off` and `filtered`, at completion.
///
/// Note that `filtered` is **not** mutually exclusive with
/// [`MpDiag::rx_accepted`]: a frame counts as accepted once it passes
/// `check_rx`, and may still be filtered later. See [`MpDiag`]'s doc comment.
#[derive(Clone, Copy, Default)]
pub struct RxDrops {
    /// `W_RXCnt` bit 15 clear: the driver has not armed reception.
    pub rx_disabled: u32,
    /// `W_PowerState` bit 9 set: the transceiver is powered down, so nothing
    /// can be received (`Wifi.cpp:1566-1567`).
    ///
    /// Kept apart from [`RxDrops::rx_disabled`] because the two say opposite
    /// things about the driver: one is "reception is not armed yet", the other
    /// is "the radio is asleep", and folding them together turned a power-down
    /// into what looked like an un-armed receiver.
    pub rx_powered_down: u32,
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
    /// An MP command or reply frame arrived on the regular RX path while this
    /// instance was not engaged in an MP session, and was skipped. Ported from
    /// melonDS's "ignore MP frames if not engaged in a MP comm" check
    /// (`Wifi.cpp:1620-1628`); see `docs/design/review_mp_local2.md` P0-5.
    pub foreign_mp: u32,
    /// The RX ring was full: the driver has not drained the previous frame.
    pub ring_full: u32,
    /// A WEP frame arrived while WEP processing is disabled.
    pub wep_off: u32,
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
    /// The highest channel this session ever resolved, `0` if none ever did.
    ///
    /// `channel == 0` alone cannot tell "the driver has not picked a channel
    /// yet" from "it picked one and then tore the radio back down", and those
    /// two point at opposite halves of the driver's state machine. A nonzero
    /// value here with `channel == 0` now is the second case.
    pub channel_ever: i32,
    /// How many times [`super::Wifi::change_channel`] changed the resolved
    /// channel, in either direction. A number that keeps climbing means the
    /// radio is being re-programmed in a loop rather than settling.
    pub channel_changes: u32,
    /// Microseconds of radio-on time spent holding a resolved channel, out of
    /// [`MpDiag::radio_us`].
    ///
    /// A driver re-uploading its RF block briefly clears the channel on real
    /// hardware too, so catching `channel == 0` in a snapshot proves nothing.
    /// The *fraction* does: a host that holds a channel 95% of the time is
    /// transmitting, one that holds it 5% of the time is stuck in an
    /// initialisation loop and its beacons are going nowhere.
    pub channel_us: u64,
    /// Microseconds the radio has been powered on, the denominator for
    /// [`MpDiag::channel_us`].
    pub radio_us: u64,
    /// Frames dropped on the way to the transport because no RF channel was
    /// resolved at the moment they were due to go out.
    ///
    /// The phase machine still completes and still raises IRQ 1, so the
    /// driver believes every one of these was transmitted. Nothing else in
    /// the counters distinguishes "sent" from "sent nowhere".
    pub tx_dropped_no_channel: u32,
    /// `RFChipType` from the firmware Wi-Fi config block (2 or 3).
    pub rf_version: u8,
    /// The two RF register ids the channel table is indexed by.
    pub rf_channel_index: [u32; 2],
    /// `true` if the parsed channel table is entirely zero -- channel
    /// detection cannot work, and either the firmware image or the parse
    /// offsets are wrong. See [`super::Wifi::load_firmware_config`].
    pub rf_table_empty: bool,
    /// `true` when the two channel-selection RF registers currently hold the
    /// firmware's `InitialRFValues` — i.e. the driver has (re-)uploaded the
    /// radio's power-on block and has not selected a channel since.
    ///
    /// This is the difference between "the radio was re-initialised" and "the
    /// game asked for a channel this firmware's table does not contain". Both
    /// leave `channel == 0`, but only the second is a channel-table problem,
    /// and reporting the second when the first happened sends the reader
    /// chasing the wrong thing entirely.
    pub rf_at_initial_values: bool,
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
    /// The association response's AID field and the guards around it, from
    /// the last one received. `Wifi::check_rx` only promotes this instance to
    /// an MP client when the frame is tagged `Packet`, carries a non-zero
    /// sender timestamp, is addressed to us, *and* grants a non-zero AID --
    /// so when an `assoc-resp` arrives and `is_mp` stays false, these say
    /// which of the four failed.
    pub last_assoc_aid: u16,
    pub last_assoc_mac_good: bool,
    pub last_assoc_is_packet: bool,
    pub last_assoc_timestamp: u64,

    /// Non-zero `answered` masks returned by the transport's reply collection.
    pub replies_answered: u32,
    /// Reply collections that returned an empty mask.
    pub replies_empty: u32,
    /// IRQ 12 (MP CMD transaction complete) raises.
    pub irq12: u32,
    /// IRQ 13 (post-beacon auto power-down) raises, and how many of those
    /// actually powered the transceiver down.
    pub irq13: u32,
    pub irq13_powered_down: u32,
    /// IRQ 15 (pre-beacon auto wake-up) raises, and how many actually woke
    /// the transceiver. A client that sleeps but is never woken shows
    /// `irq13_powered_down > 0` alongside `irq15_woke == 0` -- which is
    /// exactly the state that leaves it spinning on `W_PowerState` forever.
    pub irq15: u32,
    pub irq15_woke: u32,
    /// Times [`super::Wifi::update_power_status`] took its power-*off*
    /// branch, and the reason it did. `W_ModeReset` bit 0 is the
    /// transceiver's master enable: with it clear, melonDS forces power off
    /// unconditionally (`Wifi.cpp:481-483`), and nothing short of the driver
    /// setting that bit brings the radio back.
    pub power_off_events: u32,
    pub power_off_by_mode_reset: u32,
    /// Live `W_ModeReset` value and whether the radio currently reads as
    /// powered down (`W_PowerState` bit 9).
    pub mode_reset_reg: u16,
    pub powered_down: bool,
    /// `W_ModeWEP` and `W_PowerDownCtrl`. Between them these decide whether a
    /// powered-down radio can ever be asked to come back: `W_PowerState` is
    /// writable only in `W_ModeWEP` mode 3, and `W_PowerDownCtrl` bit 1 is
    /// the only other request path besides IRQ 15.
    pub mode_wep_reg: u16,
    pub power_down_ctrl_reg: u16,
    /// Live `W_TXSlotCmd`, `W_TXReqRead` and `W_RXCnt`. `Wifi::fire_tx`
    /// starts the MP command slot only when `W_RXCnt` bit 15 is set, the
    /// slot register has bit 15 set, and the matching `W_TXReqRead` bit is
    /// set. With CMD rounds stuck at zero these three say which of those the
    /// driver never satisfied.
    /// Writes to `W_TXSlotCmd`, and how many had their bit 15 silently
    /// dropped because `CmdCounter` was zero (`Wifi.cpp:2425-2427`). That
    /// rule is the one way an armed CMD slot can vanish without a trace, and
    /// with CMD rounds stuck at zero it is the first thing to rule out.
    /// RX-complete interrupts (IRQ 0) raised, and how many of those the
    /// driver had masked off in `W_IE` at the time.
    ///
    /// A frame this port delivers perfectly is still invisible to a game whose
    /// driver is not listening for it, and every other counter here reads
    /// identically in both cases.
    pub irq0_raised: u32,
    /// Of [`MpDiag::irq0_raised`], how many were raised with `W_IE` bit 0
    /// clear.
    pub irq0_masked: u32,
    /// Halfwords the driver read directly out of the RX ring's Wi-Fi RAM
    /// window (`4804000h`-`4805FFFh`), which is how a DS driver normally
    /// consumes a received frame.
    ///
    /// [`MpDiag::rx_ring_reads`] counts only the `W_RXBufDataRead` port, so on
    /// its own it reads zero for a perfectly healthy driver. The two together
    /// are what answer "did the game look at the frame at all".
    pub rx_ram_reads: u32,
    /// Halfwords the driver pulled out of the RX ring through
    /// `W_RXBufDataRead`.
    ///
    /// This is the other half of [`MpDiag::irq0_raised`]: interrupts raised
    /// with nothing read back means the driver never consumed the frames, so
    /// the fault is in how reception is *signalled*, not in the frames
    /// themselves. A healthy session reads back roughly one halfword per
    /// halfword delivered.
    pub rx_ring_reads: u32,
    pub tx_slot_cmd_writes: u32,
    pub tx_slot_cmd_bit15_dropped: u32,
    /// Writes to `W_CmdCount`, which is the only thing that makes
    /// `CmdCounter` non-zero.
    pub cmd_count_writes: u32,
    pub tx_slot_cmd_reg: u16,
    pub tx_req_read_reg: u16,
    pub rx_cnt_reg: u16,
    /// `Wifi::fire_tx` calls, and how many returned early because reception
    /// was not armed.
    pub fire_tx_calls: u32,
    pub fire_tx_rx_disabled: u32,

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
             [mp-diag]                     channel_mismatch={} filtered={} foreign_mp={} ring_full={}\n\
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
            d.foreign_mp,
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
        // `channel == 0` *right now* is not evidence of anything on its own:
        // a driver re-uploading its RF block clears the channel for a moment
        // on real hardware too, and a client sweeping for a room clears it
        // between every hop. Only a console that spends a substantial share of
        // its radio-on time off-channel is actually failing here.
        //
        // Reporting on the instantaneous value sent two rounds of
        // investigation after the radio while the real fault was downstream,
        // which is worse than saying nothing.
        let mostly_on_channel = self.channel_us * 10 > self.radio_us * 9;
        if self.channel == 0 && !mostly_on_channel {
            if self.rf_transfers == 0 {
                return format!(
                    "VERDICT: the driver never programmed the RF chip (0 RF transfers, {} BB \
                     writes), so no channel was ever selected. The failure is upstream of the \
                     channel table -- the game's Wi-Fi init did not get as far as the radio.",
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
            if self.rf_at_initial_values {
                return format!(
                    "VERDICT: the radio holds the firmware's InitialRFValues \
                     (RF[{}]=0x{:X} RF[{}]=0x{:X}), so the driver has (re-)initialised the RF \
                     chip and not selected a channel since. This is NOT a channel-table gap: \
                     those two values are the power-on defaults, not a channel the game asked \
                     for. If traffic flowed earlier in this session, the driver tore the radio \
                     down and restarted -- look for what made it re-initialise (a rejected \
                     association, a deauth, or a repeatedly-reprogrammed RX ring: check \
                     rxbuf_cfg).",
                    self.rf_channel_index[0],
                    self.rf_regs_now[0],
                    self.rf_channel_index[1],
                    self.rf_regs_now[1],
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
        // A powered-down radio cannot transmit at all, so say that rather
        // than blaming the driver for not arming a slot.
        if self.powered_down {
            return format!(
                "VERDICT: the transceiver is powered down (W_PowerState bit 9) and nothing is \
                 bringing it back. W_ModeWEP=0x{:04X} (W_PowerState is writable only in mode 3), \
                 W_PowerDownCtrl=0x{:04X}, W_ModeReset=0x{:04X}. Until it powers up this \
                 instance cannot transmit.",
                self.mode_wep_reg, self.power_down_ctrl_reg, self.mode_reset_reg
            );
        }
        if self.loc_tx == 0
            && self.beacon_tx == 0
            && self.cmd_tx == 0
            && self.reply_tx == 0
            && self.blank_reply_tx == 0
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
                (d.foreign_mp, "MP frames from a session this instance is not part of"),
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
                    "VERDICT: reception is armed and polling ({} times), but the peer has sent \
                     nothing at all. The problem is on the *other* instance's transmit side, or \
                     in the room/UDP layer between them.",
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
        // Traffic flows both ways but neither side ever becomes an MP peer.
        // This is the stage the counters used to fall straight through --
        // ending on "the handshake looks healthy" for a pair that never
        // associated, because every MP counter after this point is zero as a
        // *consequence* of the failure rather than as evidence against it.
        //
        // The `last_assoc_*` fields are recorded by
        // [`super::Wifi::check_rx`] before its own guards run, precisely so an
        // association response that arrived and was rejected can be told from
        // one that never came.
        if !self.is_mp && self.rx_accepted >= 4 && self.cmd_tx == 0 && self.rxflags_cmd == 0 {
            if self.last_assoc_timestamp == 0 && self.last_assoc_aid == 0 {
                return format!(
                    "VERDICT: frames are flowing ({} accepted, {} beacons, {} management) but no \
                     association response (frame type 0010) has ever arrived, so this instance \
                     never becomes an MP peer. If this is the client, the host is not answering \
                     its association request; if this is the host, it never sent one.",
                    self.rx_accepted, self.rxflags_beacon, self.rxflags_mgmt
                );
            }
            if self.last_assoc_aid == 0 {
                return "VERDICT: an association response arrived carrying AID 0 -- the host \
                        refused the association rather than granting a slot."
                    .to_string();
            }
            if !self.last_assoc_mac_good {
                return format!(
                    "VERDICT: an association response arrived granting AID {}, but its \
                     destination address is not this instance's MAC and not a broadcast, so it \
                     was ignored. The two instances' MACs are derived from their instance index; \
                     check they actually differ.",
                    self.last_assoc_aid
                );
            }
            if self.last_assoc_timestamp == 0 {
                return format!(
                    "VERDICT: an association response arrived granting AID {}, addressed \
                     correctly, but with a zero wireless timestamp. The promote path requires a \
                     non-zero one (it is the host clock the client adopts), so the association \
                     is dropped on the floor. The fault is in the transport's timestamp, not in \
                     the handshake.",
                    self.last_assoc_aid
                );
            }
        }
        // Associated at some point, but not in a session now and no MP round
        // has ever run. This is the state a pair lands in when the link comes
        // up (the game shows signal bars) and then decays -- and it used to
        // fall through to "the handshake looks healthy", because every counter
        // that would contradict that is zero *because* the round never
        // started.
        if !self.is_mp && self.last_assoc_aid != 0 && self.cmd_tx == 0 && self.rxflags_cmd == 0 {
            return format!(
                "VERDICT: an association response was accepted (AID 0x{:04X}) but no MP command \
                 round has ever run and this instance is not in a session now -- it associated \
                 and the link was then torn down. The host arms a round by writing W_TXSlotCmd \
                 with bit 15: {} such writes, {} of them had bit 15 dropped because CmdCounter \
                 was zero ({} W_CmdCount writes). W_TXSlotCmd=0x{:04X} W_TXReqRead=0x{:04X}.",
                self.last_assoc_aid,
                self.tx_slot_cmd_writes,
                self.tx_slot_cmd_bit15_dropped,
                self.cmd_count_writes,
                self.tx_slot_cmd_reg,
                self.tx_req_read_reg,
            );
        }
        // The host side of the same failure: it beacons, it answers, but it
        // never opens a round.
        if !self.is_mp && self.beacon_tx > 0 && self.cmd_tx == 0 && self.rx_accepted > 0 {
            return format!(
                "VERDICT: this instance beaconed ({} beacons) and heard {} frames back, but \
                 never armed an MP command round. W_TXSlotCmd writes: {} ({} had bit 15 dropped \
                 for CmdCounter == 0), W_CmdCount writes: {}. W_TXSlotCmd=0x{:04X} \
                 W_TXReqRead=0x{:04X}.",
                self.beacon_tx,
                self.rx_accepted,
                self.tx_slot_cmd_writes,
                self.tx_slot_cmd_bit15_dropped,
                self.cmd_count_writes,
                self.tx_slot_cmd_reg,
                self.tx_req_read_reg,
            );
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
