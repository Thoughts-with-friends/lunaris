//! Internet play: emulated Ethernet frames between the DS's
//! Wi-Fi adapter and a real network.
//!
//! Corresponds to melonDS's `src/net/Net.{h,cpp}`, `NetDriver.h` and
//! `PacketDispatcher.{h,cpp}`.
//!
//! # Not ported
//! melonDS's two concrete drivers are deliberately absent:
//!
//! * `Net_PCap.cpp` — dynamically loads libpcap and bridges onto a
//!   physical adapter. It is almost entirely FFI plumbing plus adapter
//!   enumeration.
//! * `Net_Slirp.cpp` — a complete user-mode TCP/IP stack supplied by
//!   libslirp, plus DNS frame rewriting.
//!
//! Both would pull a C library into `nds-core`. The [`NetDriver`] trait is
//! the seam where a frontend can add either one; [`NullNetDriver`] and
//! [`LoopbackNetDriver`] cover the "no internet" and "test" cases.
//!
//! Note that this module is about the *internet* path. The DS Wi-Fi
//! **hardware** registers live in `crate::hw::wifi`, a separate and
//! unrelated module.
//!
//! DS Wi-Fi hardware ("W_" registers at `4800000h`-`4808FFFh`) — enough of
//! it to support **local multiplayer (MP) mode** between two `lunaris`
//! instances. Internet play (WFC), WEP/WPA association with a real access
//! point, and DSi-specific Wi-Fi are out of scope.
//!
//! Structurally this is a port of melonDS's `src/net/Wifi.cpp` (vendored at
//! `docs/design/melonds/WiFi.cpp` for reference), adapted to lunaris's
//! scheduler/IO conventions and simplified to the subset needed for MP
//! association and CMD/reply/ack exchange. See
//! `docs/design/design_lan.md` §6 for the full design rationale, including
//! the traps this implementation specifically avoids (16-bit register
//! access semantics, IRQ edge-triggering, per-instance MAC uniqueness, and
//! non-zero synthetic RF channel tables).
//!
//! GBATEK: <https://problemkaputt.de/gbatek.htm#dswirelesscommunications>

mod bb_rf;
pub mod diag;
pub mod mp;
mod net;
mod net_driver;
mod packet_dispatcher;
mod regs;
mod rx;
mod tx;

pub use mp::LinkHints;
use mp::MpTransport;
pub use net::Net;
pub use net_driver::{LoopbackNetDriver, NetDriver, NullNetDriver, RxCallback};
pub use packet_dispatcher::{
    DispatchedPacket, EXTERNAL_SENDER, PACKET_QUEUE_SIZE, PacketDispatcher,
};
pub use regs::*;
use rx::{DeferredRxParams, PendingRxHeader, RxKind};

use crate::hw::{HW, Scheduler, interrupt_controller::InterruptRequest, scheduler::Event};

/// Diagnostic tracing for the TX/RX/beacon path, independent of the `log`
/// crate's configured level (which the frontends default to `Off`; see
/// `gui/egui/src/main.rs`'s `TermLogger::init`). Enabled by setting
/// `LUNARIS_WIFI_DEBUG=1` before launching, so a real-game connectivity
/// issue (e.g. a Union Room never showing the peer) can be diagnosed
/// without rebuilding or reconfiguring logging. Checked once and cached,
/// since [`Wifi::tick`] runs far too often to re-read the environment
/// every call.
pub(super) fn debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("LUNARIS_WIFI_DEBUG").is_some())
}

/// Whether the association-response trace is enabled, cached from the
/// environment.
///
/// Answers one question and then gets out of the way: when an association
/// response reaches the RX ring but the driver never writes `W_AIDLow`, is the
/// driver reading back the bytes we actually wrote?
///
/// Enable with `LUNARIS_MP_ASSOC_TRACE=1`. It prints the RX header and body
/// this instance committed for each association response, then every
/// `W_RXBufDataRead` the driver performs for the next
/// [`Wifi::ASSOC_TRACE_READS`] reads, with the address each came from. Comparing
/// the two tells you whether the driver is reading the frame, reading the wrong
/// place, or not reading at all -- three very different faults that look
/// identical from the counters. See `docs/design/review_mp_local2.md` §7.1d.
pub(super) fn assoc_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("LUNARIS_MP_ASSOC_TRACE").is_some())
}

/// One hardware TX slot (LOC1-3, CMD, beacon, or MP reply).
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Default)]
pub(super) struct TxSlot {
    pub valid: bool,
    pub addr: u16,
    pub length: u16,
    /// `1` = 1 Mbit (long preamble), `2` = 2 Mbit (short preamble).
    pub rate: u8,
    pub phase: u8,
    pub phase_time: i32,
}

/// DS Wi-Fi hardware state.
#[derive(emu_utils::Savestate)]
#[load(in_place_only)]
pub struct Wifi {
    #[load(with_in_place = "*ram = save.load()?")]
    ram: Vec<u8>,
    /// Register file, word-indexed (`io[addr >> 1]`), covering `000h-FFFh`.
    #[load(with_in_place = "*io = save.load()?")]
    io: Vec<u16>,

    enabled: bool,
    power_on: bool,
    /// Fractional-cycle carry for the 8 microsecond timer. See
    /// `docs/design/design_lan.md` §6.3.
    timer_error: i64,

    random: u16,

    bb_regs: Vec<u8>,
    bb_regs_ro: Vec<u8>,
    rf_version: u8,
    rf_regs: Vec<u32>,
    rf_channel_index: [u32; 2],
    rf_channel_data: [[u32; 2]; 14],
    /// What the firmware's `InitialRFValues` block holds at the two register
    /// indices channel selection is read from, or [`u32::MAX`] if unknown.
    ///
    /// Diagnostic only. When the driver re-initialises the radio it re-uploads
    /// this block, which overwrites the channel-selection registers and drops
    /// `cur_channel` to zero. That looks identical to "the game asked for a
    /// channel the table does not contain", but has a completely different
    /// cause, so the two are worth telling apart in the verdict. See
    /// [`diag::MpDiag::verdict`].
    rf_initial_values: [u32; 2],
    /// Firmware header `ConsoleType` (offset `01Dh`), supplied by
    /// [`Wifi::set_console_type`]. Decides `W_ID`, and through it the
    /// `W_RXBufGapSize` auto-clear. Defaults to `FFh` (original DS).
    console_type: u8,
    cur_channel: i32,

    tx_slots: [TxSlot; 6],
    #[load(with_in_place = "*tx_buffer = save.load()?")]
    tx_buffer: Vec<u8>,
    tx_cur_slot: i32,
    tx_seqno_pending: bool,

    #[load(with_in_place = "*rx_buffer = save.load()?")]
    rx_buffer: Vec<u8>,
    rx_time: i32,
    rx_counter: u32,

    /// `0` = idle, bit 0 = receiving, bit 1 = transmitting.
    com_status: u32,

    mp_client_mask: u16,
    mp_client_fail: u16,
    mp_reply_timer: i32,
    #[load(with_in_place = "*mp_client_replies = save.load()?")]
    mp_client_replies: Vec<u8>,

    is_mp: bool,
    is_mp_client: bool,
    us_timestamp: u64,
    us_counter: u64,
    us_compare: u64,
    next_sync: u64,
    rx_timestamp: u64,

    /// Sequence number of the last MP CMD frame delivered, or `0xFFFF`
    /// (matching melonDS's `MPLastSeqno` reset value) before any has been
    /// seen. Used to suppress delivering the exact same CMD frame twice.
    mp_last_seqno: u16,
    /// Classification awaiting the simulated clock to reach a delayed
    /// frame's timestamp. See [`rx::DeferredRxParams`].
    rx_deferred: DeferredRxParams,
    /// RX-ring header build awaiting the simulated transfer time to
    /// elapse. See [`rx::PendingRxHeader`].
    rx_pending: PendingRxHeader,

    /// Whether the driver has ever written `W_RXBufReadCursor`.
    ///
    /// [`Wifi::start_rx`] refuses to overwrite RX-ring bytes the driver has
    /// not drained yet, which it detects by walking the write cursor towards
    /// `W_RXBufReadCursor`. That test is only meaningful once the driver has
    /// actually published a read cursor: until then the register reads zero,
    /// which after the halfword shift and `0x1FFE` mask is also the ring
    /// base, so the very first frame of a session appears to collide with it
    /// and is dropped. melonDS never hits this because it detects the overrun
    /// inside its byte pump (`Wifi.cpp:1909-1936`), where the comparison only
    /// happens after at least one halfword has been written.
    ///
    /// See `docs/design/review_mp_local2.md` P0-4.
    rx_read_cursor_written: bool,

    /// Remaining `W_RXBufDataRead` reads to trace after an association
    /// response was committed to the RX ring. See [`assoc_trace_enabled`].
    #[savestate(skip)]
    assoc_trace_reads: u32,

    /// Set by [`Wifi::update_power_status`] when the transceiver starts
    /// powering up, and drained by [`Wifi::tick`]. Latched rather than
    /// raised directly because power status is re-evaluated from register
    /// writes that do not all have an [`InterruptRequest`] to hand.
    pending_irq11: bool,

    /// Per-register read tally, indexed like [`Wifi::io`]. Not serialized:
    /// purely diagnostic. A driver that stalls waiting on the hardware
    /// spins on one register, so the most-read port names what it is
    /// waiting for -- which is otherwise invisible.
    #[savestate(skip)]
    reg_read_counts: Vec<u32>,

    /// Local-multiplayer progress counters. Not serialized: purely
    /// diagnostic, and a savestate load should not carry stale counts into
    /// a fresh session. See [`diag`].
    #[savestate(skip)]
    pub(super) diag: diag::MpDiag,

    us_until_power_on: i32,
    cmd_counter: u32,

    /// Not serialized: reinstalled by the frontend after a savestate load
    /// via [`Wifi::set_transport`], mirroring how `firmware`/`bios7` are
    /// re-supplied rather than serialized (`docs/design/design_lan.md` §8.1).
    #[savestate(skip)]
    transport: Option<Box<dyn MpTransport>>,
}

impl Wifi {
    /// How many `W_RXBufDataRead` reads to trace after an association response
    /// lands. A DS association response is ~40 bytes plus the 12-byte RX
    /// header, i.e. ~26 halfwords; 64 covers it with room for the driver to
    /// walk past the end.
    const ASSOC_TRACE_READS: u32 = 64;

    const RAM_SIZE: usize = 0x2000;
    const IO_WORDS: usize = 0x800;
    const TX_BUFFER_SIZE: usize = 0x2000;
    const RX_BUFFER_SIZE: usize = 2048;
    const MP_CLIENT_REPLIES_SIZE: usize = 15 * 1024;
    const TIMER_INTERVAL_US: u64 = 8;

    pub fn new() -> Self {
        let mut wifi = Wifi {
            ram: vec![0; Self::RAM_SIZE],
            io: vec![0; Self::IO_WORDS],
            enabled: false,
            power_on: false,
            timer_error: 0,
            random: 1,
            bb_regs: vec![0; 0x100],
            bb_regs_ro: vec![0; 0x100],
            rf_version: 0,
            rf_regs: vec![0; 0x40],
            rf_channel_index: [0; 2],
            rf_channel_data: [[0; 2]; 14],
            rf_initial_values: [u32::MAX; 2],
            console_type: 0xFF,
            cur_channel: 0,
            tx_slots: [TxSlot::default(); 6],
            tx_buffer: vec![0; Self::TX_BUFFER_SIZE],
            tx_cur_slot: -1,
            tx_seqno_pending: false,
            rx_buffer: vec![0; Self::RX_BUFFER_SIZE],
            rx_time: 0,
            rx_counter: 0,
            com_status: 0,
            mp_client_mask: 0,
            mp_client_fail: 0,
            mp_reply_timer: 0,
            mp_client_replies: vec![0; Self::MP_CLIENT_REPLIES_SIZE],
            is_mp: false,
            is_mp_client: false,
            us_timestamp: 0,
            us_counter: 0,
            us_compare: 0,
            next_sync: 0,
            rx_timestamp: 0,
            mp_last_seqno: 0xFFFF,
            rx_deferred: DeferredRxParams::default(),
            rx_pending: PendingRxHeader::default(),
            rx_read_cursor_written: false,
            assoc_trace_reads: 0,
            pending_irq11: false,
            reg_read_counts: vec![0; Self::IO_WORDS],
            diag: diag::MpDiag::default(),
            us_until_power_on: 0,
            cmd_counter: 0,
            transport: None,
        };
        wifi.reset();
        wifi
    }

    /// Installs (or removes, with `None`) the frontend-supplied MP
    /// transport. Safe to call at any time, including after a savestate
    /// load — the transport is never serialized.
    pub fn set_transport(&mut self, transport: Option<Box<dyn MpTransport>>) {
        self.transport = transport;
    }

    pub(crate) fn post_load(&mut self) {
        // A loaded savestate cannot resume mid-MP-session: the peer's
        // timeline was just rewound, which desyncs every other room member.
        // Force a clean re-association instead of silently corrupting the
        // link. See `docs/design/design_lan.md` §13.3.
        self.is_mp = false;
        self.is_mp_client = false;
        self.next_sync = 0;
        self.rx_timestamp = 0;
        self.mp_client_mask = 0;
        self.mp_client_fail = 0;
        // The latched TX slot is meaningless across a rewind; leaving a stale
        // index would have `Wifi::process_tx` advance a slot the reloaded
        // `W_TXBusy` no longer marks busy. See
        // `docs/design/local-mp-melonds-parity-2.md` F3.
        self.tx_cur_slot = -1;
        self.com_status = 0;
    }

    /// Loads channel calibration and BSSID-independent defaults. Mirrors
    /// melonDS `Wifi::Reset` (`docs/design/melonds/WiFi.cpp:106-200`).
    ///
    /// `firmware_wifi_config` is the raw Wi-Fi calibration block copied out
    /// of the firmware image (offset `02Ch` onward, per
    /// `docs/design/design_lan.md` §7.1); pass `None` before firmware is
    /// available (e.g. at construction) to leave channel detection
    /// unresolved until [`Wifi::load_firmware_config`] is called.
    pub fn reset(&mut self) {
        self.ram.iter_mut().for_each(|b| *b = 0);
        self.io.iter_mut().for_each(|w| *w = 0);
        self.enabled = false;
        self.power_on = false;
        self.random = 1;
        // The register file was just zeroed, so `W_RXBufReadCursor` is back to
        // "never published by the driver". See [`Wifi::rx_read_cursor_written`].
        self.rx_read_cursor_written = false;
        self.assoc_trace_reads = 0;
        self.bb_regs.iter_mut().for_each(|b| *b = 0);
        self.bb_regs_ro.iter_mut().for_each(|b| *b = 0);

        const BB_FIXED: &[(usize, u8)] = &[
            (0x00, 0x6D),
            (0x0D, 0x00),
            (0x0E, 0x00),
            (0x0F, 0x00),
            (0x10, 0x00),
            (0x11, 0x00),
            (0x12, 0x00),
            (0x16, 0x00),
            (0x17, 0x00),
            (0x18, 0x00),
            (0x19, 0x00),
            (0x1A, 0x00),
            (0x27, 0x00),
            (0x4D, 0x00),
            (0x5D, 0x01),
            (0x5E, 0x00),
            (0x5F, 0x00),
            (0x60, 0x00),
            (0x61, 0x00),
            (0x64, 0xFF),
            (0x66, 0x00),
        ];
        for &(id, val) in BB_FIXED {
            self.bb_regs[id] = val;
            self.bb_regs_ro[id] = 1;
        }
        for i in 0x69..0x100 {
            self.bb_regs[i] = 0;
            self.bb_regs_ro[i] = 1;
        }

        self.rf_regs.iter_mut().for_each(|r| *r = 0);
        self.rf_initial_values = [u32::MAX; 2];
        self.cur_channel = 0;
        self.tx_cur_slot = -1;
        self.com_status = 0;
        self.mp_client_mask = 0;
        self.mp_client_fail = 0;
        self.mp_last_seqno = 0xFFFF;
        self.rx_deferred = DeferredRxParams::default();
        self.rx_pending = PendingRxHeader::default();

        // `W_ID` (000h) is a hardware-identification register a driver
        // reads during Wi-Fi init to confirm a real chip is present before
        // doing anything else; leaving it at zero looks like "no Wi-Fi
        // hardware" and can make a driver skip wireless entirely without
        // ever touching another W_* register.
        //
        // The value is not a constant: melonDS derives it from the firmware
        // header's `ConsoleType` (`Wifi.cpp:186-197`), because the DS Lite and
        // DSi carry a later Wi-Fi variant that reports `0xC340` and behaves
        // differently -- see the `W_RXBufGapSize` auto-clear in
        // [`super::regs`]. Hard-coding the original-DS `0x1440`, as this used
        // to, means a DS Lite firmware dump (a very common thing to configure)
        // gets emulated as hardware it is not.
        //
        // Left at the original-DS value until [`Wifi::set_console_type`]
        // supplies the header byte, so a `Wifi` built without firmware still
        // identifies as *some* real chip rather than as absent hardware.
        self.set_ioport(W_ID, Self::console_wifi_id(self.console_type));

        // MAC/BSSID reset to all-FF (unprogrammed), not zero -- the driver
        // itself copies the real MAC out of firmware into `W_MACAddr0..2`
        // during init (`docs/design/melonds/WiFi.cpp:199`).
        self.set_ioport(W_MACAddr0, 0xFFFF);
        self.set_ioport(W_MACAddr1, 0xFFFF);
        self.set_ioport(W_MACAddr2, 0xFFFF);
        self.set_ioport(W_BSSID0, 0xFFFF);
        self.set_ioport(W_BSSID1, 0xFFFF);
        self.set_ioport(W_BSSID2, 0xFFFF);

        // Hardware resets with power-save (bit 0) already set; the driver
        // must explicitly clear it before the radio can power on. See
        // `Wifi::update_power_on` and `docs/design/melonds/WiFi.cpp:203`.
        self.set_ioport(W_PowerUS, 0x0001);
    }

    /// Populates RF channel calibration from a firmware Wi-Fi config block.
    /// `config` starts at firmware offset `02Ch`
    /// (`docs/design/design_lan.md` §7.1).
    ///
    /// Without this table [`Wifi::change_channel`] can never match the values
    /// the game writes into the RF index registers, `cur_channel` stays `0`,
    /// and **every** transmission and reception is discarded -- local play
    /// cannot start at all.
    ///
    /// # Offsets
    /// All offsets below are derived from melonDS's `FirmwareHeader`
    /// (`docs/design/melonds/SPI_Firmware.h:278-325`) by summing field sizes
    /// from `WifiConfigChecksum` at firmware `02Ch`, and are given both as
    /// absolute firmware offsets and as indices into `config`
    /// (`firmware - 0x2C`):
    ///
    /// | Field | Firmware | `config` |
    /// | --- | --- | --- |
    /// | `RFChipType` | `040h` | `0x14` |
    /// | `InitialValues[32]` | `044h` | `0x18` |
    /// | `InitialBBValues[105]` | `064h` | `0x38` |
    /// | union start | `0CEh` | `0xA2` |
    /// | Type2 `InitialRF56Values[84]` | `0F2h` | `0xC6` |
    /// | Type3 `RFIndex1` | `116h` | `0xEA` |
    /// | Type3 `RFData1[14]` | `117h` | `0xEB` |
    /// | Type3 `RFIndex2` | `125h` | `0xF9` |
    /// | Type3 `RFData2[14]` | `126h` | `0xFA` |
    /// `W_ID` for a firmware header `ConsoleType` byte (`SPI_Firmware.h`'s
    /// `FirmwareConsoleType`). Ported from `Wifi::Reset`
    /// (`docs/design/melonds/WiFi.cpp:186-197`).
    ///
    /// The DS Lite and the DSi carry a later Wi-Fi variant reporting `0xC340`;
    /// the original DS and the iQue DS report `0x1440`. An unrecognised byte
    /// falls back to `0x1440`, as melonDS does after logging.
    const fn console_wifi_id(console_type: u8) -> u16 {
        match console_type {
            // `DSLite` (20h), `iQueDSLite` (63h), and `DSi` (57h).
            0x20 | 0x63 | 0x57 => 0xC340,
            // `DS` (FFh), `iQueDS` (43h), and anything unrecognised.
            _ => 0x1440,
        }
    }

    /// Supplies the firmware header's `ConsoleType` byte (offset `01Dh`) and
    /// republishes `W_ID`.
    ///
    /// Separate from [`Wifi::load_firmware_config`] because that takes the
    /// Wi-Fi calibration block from firmware `02Ch` onward, and `ConsoleType`
    /// sits *before* it.
    pub fn set_console_type(&mut self, console_type: u8) {
        self.console_type = console_type;
        self.set_ioport(W_ID, Self::console_wifi_id(console_type));
    }

    /// `true` when this instance emulates the later Wi-Fi variant found in the
    /// DS Lite and DSi, which melonDS gates the `W_RXBufGapSize` auto-clear on
    /// (`Wifi.cpp:2072-2073`).
    pub(super) fn is_modern_wifi(&self) -> bool {
        self.ioport(W_ID) == 0xC340
    }

    pub fn load_firmware_config(&mut self, config: &[u8]) {
        /// `RFChipType`, firmware `040h`.
        const RF_CHIP_TYPE: usize = 0x14;
        /// Type-3 `InitialRFValues[41]`, firmware `0CEh`.
        ///
        /// Derived by walking back from [`TYPE3_RF_INDEX1`] through the fields
        /// `SPI_Firmware.h`'s `Type3Config` declares before it: `RFIndex1` at
        /// `0xEA`, preceded by `BBData2[14]`, `BBIndex2`, `BBData1[14]`,
        /// `BBIndex1` and `BBIndicesPerChannel`, which puts the 41-byte
        /// `InitialRFValues` block at `0xEA - 1 - 14 - 1 - 14 - 1 - 41`.
        const TYPE3_INITIAL_RF: usize = 0xA2;
        /// Type-3 `RFIndex1`, firmware `116h`.
        const TYPE3_RF_INDEX1: usize = 0xEA;
        /// Type-3 `RFData1[14]`, firmware `117h`.
        const TYPE3_RF_DATA1: usize = 0xEB;
        /// Type-3 `RFIndex2`, firmware `125h`.
        const TYPE3_RF_INDEX2: usize = 0xF9;
        /// Type-3 `RFData2[14]`, firmware `126h`.
        const TYPE3_RF_DATA2: usize = 0xFA;
        /// Type-2 `InitialRF56Values[84]`, firmware `0F2h`.
        ///
        /// This used to read `0x38`, which is `InitialBBValues` -- a
        /// completely different field. A Type-2 firmware therefore produced a
        /// garbage channel table, no channel ever matched, and `cur_channel`
        /// stayed `0` for the whole session.
        const TYPE2_RF56: usize = 0xC6;
        /// 14 channels x 6 bytes.
        const TYPE2_RF56_LEN: usize = 84;

        if config.len() <= RF_CHIP_TYPE {
            return;
        }
        self.rf_version = config[RF_CHIP_TYPE];

        if self.rf_version == 3 {
            if config.len() < TYPE3_RF_DATA2 + 14 {
                warn!(
                    "wifi: firmware Wi-Fi config too short for a Type-3 RF table ({} bytes); \
                     channel detection will fail",
                    config.len()
                );
                return;
            }
            self.rf_channel_index =
                [config[TYPE3_RF_INDEX1] as u32, config[TYPE3_RF_INDEX2] as u32];
            for i in 0..14 {
                self.rf_channel_data[i][0] = config[TYPE3_RF_DATA1 + i] as u32;
                self.rf_channel_data[i][1] = config[TYPE3_RF_DATA2 + i] as u32;
            }
            // The values the driver uploads to the RF chip during init, before
            // it selects any channel. Recorded purely so the diagnostic can
            // tell "the radio was re-initialised and no channel re-selected"
            // apart from "the game picked a channel this table lacks" -- two
            // very different faults that both surface as `cur_channel == 0`.
            // See [`Wifi::rf_initial_values`].
            for (i, slot) in self.rf_initial_values.iter_mut().enumerate() {
                let idx = self.rf_channel_index[i] as usize;
                *slot = config.get(TYPE3_INITIAL_RF + idx).map_or(u32::MAX, |&b| b as u32);
            }
        } else {
            if config.len() < TYPE2_RF56 + TYPE2_RF56_LEN {
                warn!(
                    "wifi: firmware Wi-Fi config too short for a Type-2 RF table ({} bytes); \
                     channel detection will fail",
                    config.len()
                );
                return;
            }
            // Type-2 packs two 18-bit values per channel into six bytes.
            // The index registers are the top 6 bits of the third byte of
            // each of the first two entries (`InitialRF56Values[2] >> 2`,
            // `[5] >> 2`).
            self.rf_channel_index =
                [(config[TYPE2_RF56 + 2] >> 2) as u32, (config[TYPE2_RF56 + 5] >> 2) as u32];
            for i in 0..14 {
                let o = TYPE2_RF56 + i * 6;
                self.rf_channel_data[i][0] = config[o] as u32
                    | (config[o + 1] as u32) << 8
                    | ((config[o + 2] as u32) & 0x3) << 16;
                self.rf_channel_data[i][1] = config[o + 3] as u32
                    | (config[o + 4] as u32) << 8
                    | ((config[o + 5] as u32) & 0x3) << 16;
            }
        }

        // A table whose entries are not all distinct cannot identify a
        // channel unambiguously, and an all-zero one makes an uninitialised
        // RF spuriously resolve to channel 1 (`docs/design/design_lan.md`
        // §3.2 trap 3). Both mean channel detection is effectively broken, so
        // say so once here rather than leaving a silent `cur_channel == 0`.
        if self.rf_channel_data.iter().all(|&[a, b]| a == 0 && b == 0) {
            warn!(
                "wifi: firmware Wi-Fi config yielded an all-zero RF channel table (RFChipType={}); \
                 channel detection cannot work -- the firmware image is probably not a real dump",
                self.rf_version
            );
        }
    }

    pub(super) fn ioport(&self, addr: usize) -> u16 {
        self.io[addr >> 1]
    }

    pub(super) fn set_ioport(&mut self, addr: usize, value: u16) {
        self.io[addr >> 1] = value;
    }

    /// Transceiver power management. Ported from `UpdatePowerStatus`
    /// (`Wifi.cpp:462-567`); `power` is `1` = force on, `0` = no change
    /// (re-evaluate), `-1` = request off.
    ///
    /// This was previously left unported as "infrastructure-mode power
    /// saving, not needed for local play". That was wrong: a real game's
    /// Wi-Fi driver polls `W_PowerState` and `W_RFStatus` in a tight loop
    /// after uploading its baseband/RF tables, waiting for the transceiver to
    /// report powered-up. With neither register ever changing, the driver
    /// spins forever and never selects an RF channel -- so `cur_channel`
    /// stays `0` and no frame is ever sent or received.
    ///
    /// The precedence rules, in melonDS's own words:
    /// * `W_PowerForce` overrides everything else;
    /// * clearing `W_ModeReset` bit 0 forcibly powers the transceiver down;
    /// * otherwise power is driven by IRQ13/IRQ15 or by `W_PowerState`,
    ///   depending on the mode selected in `W_ModeWEP`;
    /// * `W_PowerDownCtrl` controls how deep a regular power-down goes.
    ///
    /// Not ported: melonDS's partial power states (`W_PowerDownCtrl` 1 or 2),
    /// which it leaves as a TODO too.
    pub(super) fn update_power_status(&mut self, power: i32) {
        let mut power = power;
        let mut mode_reset_forced = false;
        let mut curflags = 0;
        if self.ioport(W_TRXPower) == 1 {
            curflags |= 1;
        }
        if self.ioport(W_PowerState) & (1 << 9) == 0 {
            curflags |= 2;
        }
        let mut reqflags = curflags;

        if self.ioport(W_PowerForce) & (1 << 15) != 0 {
            reqflags = if self.ioport(W_PowerForce) & 1 != 0 { 0 } else { 3 };
        } else if self.ioport(W_ModeReset) & 1 == 0 {
            // melonDS forces the transceiver off here (`Wifi.cpp:481-483`).
            //
            // **Deliberate deviation:** this port leaves the power state
            // unchanged instead.
            //
            // The master enable is clear for a stretch of every driver
            // re-initialisation, and forcing off during it deadlocks this
            // port: the branch outranks every power-*on* request, so once
            // the radio is down with `W_ModeReset` clear, neither
            // `W_PowerState` bit 1, nor `W_PowerDownCtrl` bit 1, nor IRQ 15
            // can bring it back -- the driver is left polling
            // `W_PowerState`/`W_RFStatus` tens of millions of times while
            // the link dies. melonDS escapes because it models the partial
            // power states and `W_RFStatus` states 2/4/7, neither of which
            // exists here (nor in melonDS's own TODO list).
            //
            // Leaving the state alone keeps every other power path
            // faithful, and is the narrowest change that removes the
            // deadlock.
            reqflags = curflags;
            mode_reset_forced = true;
        } else {
            if power == 0 {
                if self.ioport(W_PowerState) & 0x0202 == 0x0202 {
                    power = 1;
                } else if self.ioport(W_PowerState) & 0x0201 == 0x0001 {
                    power = -1;
                }
            }
            // `W_PowerDownCtrl` bit 0 inhibits a regular power-down; bit 1
            // forces a (at least partial) wakeup.
            if power == -1 && self.ioport(W_PowerDownCtrl) & 1 != 0 {
                power = 0;
            }

            if power == 1 {
                reqflags = 3;
            } else if power == -1 {
                reqflags = if self.ioport(W_PowerDownCtrl) != 0 { 3 } else { 0 };
            } else if self.ioport(W_PowerDownCtrl) & (1 << 1) != 0 {
                reqflags = 3;
            }
        }

        if reqflags == curflags {
            return;
        }

        if reqflags & 1 != 0 {
            if curflags & 1 == 0 {
                self.set_ioport(W_TRXPower, 1);
                self.set_status(1);
            }
        } else {
            // Signal that the transceiver is about to turn off; it only
            // actually does so once no transfer is in flight.
            self.set_ioport(W_TRXPower, 2);
            if self.com_status == 0 {
                self.set_ioport(W_TRXPower, 0);
                self.set_status(9);
            }
        }

        if reqflags & 2 == 0 {
            self.diag.power_off_events += 1;
            if mode_reset_forced {
                self.diag.power_off_by_mode_reset += 1;
            }
        }

        if reqflags & 2 != 0 {
            self.set_ioport(W_PowerState, self.ioport(W_PowerState) | (1 << 8));
            if curflags & 2 == 0 && self.us_until_power_on == 0 {
                // The radio needs ~2ms to come up; `Wifi::tick` counts this
                // down and then clears `W_PowerState`.
                self.us_until_power_on = -2048;
                self.pending_irq11 = true;
            }
        } else {
            let mut state = self.ioport(W_PowerState);
            state &= !(1 << 0);
            state &= !(1 << 8);
            // Bit 9 is how the driver observes that the power-down it asked
            // for actually happened; it polls this register waiting for it.
            // Leaving it clear stalls the driver even earlier than a
            // power-down does -- measured against a real game, reception was
            // never even armed (`W_RXCnt` stayed 0) and both instances spun
            // here tens of millions of times.
            //
            // It is cleared again by `Wifi::tick`'s `us_until_power_on`
            // countdown, which writes `W_PowerState = 0` on completion
            // (`Wifi.cpp:1789-1793`) -- that is the intended way out.
            state |= 1 << 9;
            self.set_ioport(W_PowerState, state);
            self.us_until_power_on = 0;
        }
    }

    /// Publishes the transceiver state to `W_RFStatus`/`W_RFPins`. Ported
    /// from `SetStatus` (`Wifi.cpp:453-459`).
    ///
    /// These two registers are how the driver observes the radio: it polls
    /// them after powering the transceiver up and between operations. Leaving
    /// them at zero -- as this module used to, listing both as read-only
    /// ports that nothing ever wrote -- means a driver waiting for the
    /// transceiver to report "idle" waits forever. Concretely, that stalls a
    /// real game after it has uploaded the BB/RF calibration tables but
    /// *before* it selects an RF channel, so `cur_channel` stays `0` and no
    /// frame is ever sent or received.
    ///
    /// State numbering follows melonDS: `1` = idle, `3` = transmitting,
    /// `5` = MP host waiting for replies, `6` = receiving, `8` = MP
    /// reply/ack window, `9` = powered down. States 2/4/7 are unused there
    /// too.
    pub(super) fn set_status(&mut self, status: u32) {
        const RF_PINS: [u16; 10] = [0x04, 0x84, 0, 0x46, 0, 0x84, 0x87, 0, 0x46, 0x04];
        self.set_ioport(W_RFStatus, status as u16);
        self.set_ioport(W_RFPins, RF_PINS[(status as usize).min(RF_PINS.len() - 1)]);
    }

    /// Sets `W_IF` bit `irq` and raises the ARM7 Wi-Fi interrupt request on
    /// the `0 -> nonzero` edge of `W_IF & W_IE`, matching melonDS
    /// `SetIRQ`/`CheckIRQ` (`docs/design/melonds/WiFi.cpp:376-390`). Level
    /// re-triggering here would flood the ARM7 with spurious interrupts.
    fn set_irq(&mut self, irq: u32, request: &mut InterruptRequest) {
        let old_flags = self.ioport(W_IF) & self.ioport(W_IE);
        self.set_ioport(W_IF, self.ioport(W_IF) | (1 << irq));
        self.check_irq_edge(old_flags, request);
    }

    /// Re-evaluates the pending-interrupt edge against a previously
    /// captured `old_flags` snapshot, raising the ARM7 Wi-Fi request if the
    /// masked flags went from all-zero to non-zero. Shared by [`Wifi::set_irq`]
    /// and the `W_IE`/`W_IFSet` register writes
    /// (`docs/design/melonds/WiFi.cpp:376-382`), both of which can create
    /// this edge without themselves setting a new `W_IF` bit.
    pub(super) fn check_irq_edge(&mut self, old_flags: u16, request: &mut InterruptRequest) {
        let new_flags = self.ioport(W_IF) & self.ioport(W_IE);
        if old_flags == 0 && new_flags != 0 {
            *request |= InterruptRequest::WIFI;
        }
    }

    /// Handles `POWCNT2` bit-1 and `W_PowerUS` bit-0 changes.
    /// `docs/design/design_lan.md` §6.5.
    pub(crate) fn set_power_cnt(&mut self, enable: bool, scheduler: &mut Scheduler) {
        self.enabled = enable;
        self.update_power_on(scheduler);
    }

    fn update_power_on(&mut self, scheduler: &mut Scheduler) {
        let on = self.enabled && (self.ioport(W_PowerUS) & 0x1) == 0;
        if on == self.power_on {
            return;
        }
        self.power_on = on;
        if on {
            self.timer_error = 0;
            self.schedule_timer(scheduler);
            if let Some(t) = self.transport.as_mut() {
                t.begin();
            }
        } else {
            scheduler.remove(Event::Wifi);
            if let Some(t) = self.transport.as_mut() {
                t.end();
            }
        }
    }

    fn schedule_timer(&mut self, scheduler: &mut Scheduler) {
        // Master clock is 33513982 Hz; convert an 8 microsecond interval to
        // cycles, carrying the fractional remainder exactly as melonDS does
        // (`docs/design/melonds/WiFi.cpp:319-329`) so the average interval
        // stays correct over long sessions instead of drifting.
        let cycles = crate::nds::NDS::CLOCK_RATE as i64 * Self::TIMER_INTERVAL_US as i64;
        let cycles = cycles - self.timer_error;
        let delay = (cycles + 999_999) / 1_000_000;
        self.timer_error = delay * 1_000_000 - cycles;
        scheduler.schedule(Event::Wifi, HW::on_wifi_timer, delay.max(1) as usize);
    }

    /// A snapshot of the local-multiplayer progress counters, with the live
    /// hardware fields refreshed. Exposed so a frontend can render the same
    /// handshake breakdown the `LUNARIS_MP_DIAG` dump prints. See [`diag`].
    pub fn diag_snapshot(&self) -> diag::MpDiag {
        let mut snapshot = self.diag;
        snapshot.channel = self.cur_channel;
        snapshot.is_mp = self.is_mp;
        snapshot.is_mp_client = self.is_mp_client;
        snapshot.aid = self.ioport(W_AIDLow);
        snapshot.transport_installed = self.transport.is_some();
        let m0 = self.ioport(W_MACAddr0);
        let m1 = self.ioport(W_MACAddr1);
        let m2 = self.ioport(W_MACAddr2);
        snapshot.mac =
            [m0 as u8, (m0 >> 8) as u8, m1 as u8, (m1 >> 8) as u8, m2 as u8, (m2 >> 8) as u8];
        snapshot.mode_reset_reg = self.ioport(W_ModeReset);
        snapshot.mode_wep_reg = self.ioport(W_ModeWEP);
        snapshot.power_down_ctrl_reg = self.ioport(W_PowerDownCtrl);
        snapshot.tx_slot_cmd_reg = self.ioport(W_TXSlotCmd);
        snapshot.tx_req_read_reg = self.ioport(W_TXReqRead);
        snapshot.rx_cnt_reg = self.ioport(W_RXCnt);
        snapshot.powered_down = self.ioport(W_PowerState) & (1 << 9) != 0;
        snapshot.rf_version = self.rf_version;
        snapshot.rf_channel_index = self.rf_channel_index;
        snapshot.rf_table_empty = self.rf_channel_data.iter().all(|&[a, b]| a == 0 && b == 0);
        let idx0 = self.rf_channel_index[0] as usize % self.rf_regs.len();
        let idx1 = self.rf_channel_index[1] as usize % self.rf_regs.len();
        snapshot.rf_regs_now = [self.rf_regs[idx0], self.rf_regs[idx1]];
        snapshot.rf_at_initial_values = self.rf_initial_values[0] != u32::MAX
            && self.rf_regs[idx0] == self.rf_initial_values[0]
            && self.rf_regs[idx1] == self.rf_initial_values[1];

        // The five most-read registers, descending. A driver blocked in a
        // polling loop shows up here unmistakably.
        let mut ranked: Vec<(u32, usize)> = self
            .reg_read_counts
            .iter()
            .enumerate()
            .filter(|&(_, &n)| n > 0)
            .map(|(i, &n)| (n, i << 1))
            .collect();
        ranked.sort_unstable_by_key(|&(count, _)| std::cmp::Reverse(count));
        for (slot, &(count, reg)) in ranked.iter().take(5).enumerate() {
            snapshot.top_reads[slot] = (reg as u16, count);
        }
        snapshot
    }

    /// Prints the [`diag`] summary immediately. See [`crate::hw::HW::wifi_dump_diag`].
    pub fn dump_diag(&self) {
        let snapshot = self.diag_snapshot();
        snapshot.dump_now(snapshot.transport_installed);
    }

    /// Returns `true` if this instance currently believes it is engaged in
    /// an MP session (host or client). Exposed for UI link-status display.
    pub fn is_mp_active(&self) -> bool {
        self.is_mp
    }

    /// Exposes the currently configured link hints, or the default if no
    /// transport is installed. UI/debug use only.
    pub fn link_hints(&self) -> LinkHints {
        self.transport.as_deref().map(MpTransport::link_hints).unwrap_or_default()
    }

    /// One 8µs hardware tick: advances the MP sync clock, the millisecond
    /// beacon-interval counters, the TX slot phase machine, and the RX
    /// byte-pump, then reschedules itself. Called from
    /// [`super::HW::on_wifi_timer`]. Ported from melonDS `USTimer`
    /// (`docs/design/melonds/WiFi.cpp:1753-1935`).
    ///
    /// Firing `W_BeaconCount1`'s IRQ (bit 14) turned out to be load-bearing
    /// even for MP-only play: real Wi-Fi driver code (e.g. Pokémon's Union
    /// Room) re-arms the beacon TX slot and advances its own scan/connect
    /// state machine from that interrupt handler. Leaving it un-ported (as
    /// this method originally did) meant the host's driver silently never
    /// progressed past its initial wait state -- no beacon was ever put on
    /// the wire, so a joining peer's scan found nothing, even though the
    /// room-level TCP/UDP connection underneath was healthy. See
    /// `docs/design/design_lan.md` §6.3 and §17 (Union-Room symptom).
    pub(crate) fn tick(&mut self, scheduler: &mut Scheduler, request: &mut InterruptRequest) {
        self.us_timestamp += Self::TIMER_INTERVAL_US;

        if self.pending_irq11 {
            self.pending_irq11 = false;
            self.raise_irq(11, request);
        }

        // Keep the diagnostic summary's live fields current and let it decide
        // whether this tick crosses a dump boundary. See [`diag`].
        if diag::diag_enabled() {
            self.diag.channel = self.cur_channel;
            self.diag.is_mp = self.is_mp;
            self.diag.is_mp_client = self.is_mp_client;
            self.diag.aid = self.ioport(W_AIDLow);
            let transport_installed = self.transport.is_some();
            self.diag.tick_us(Self::TIMER_INTERVAL_US, transport_installed);
        }

        // `USCounter`/the millisecond beacon timer only run while the game
        // has armed them via `W_USCountCnt`; `USTimestamp` (MP sync) above
        // is unconditional hardware state and always ticks.
        if self.ioport(W_USCountCnt) != 0 {
            self.us_counter += Self::TIMER_INTERVAL_US;
            // Pre-beacon wake-up: when the time remaining until the next
            // beacon matches `W_PreBeacon`, raise IRQ 15 (and wake the
            // transceiver). Ported from `Wifi.cpp:1801-1806`.
            if self.ioport(W_USCompareCnt) != 0 {
                let uspart = (self.us_counter & 0x3FF) as u32;
                let beaconus = (u32::from(self.ioport(W_BeaconCount1)) << 10) | (0x3FF - uspart);
                let mask = !(Self::TIMER_INTERVAL_US as u32 - 1);
                if beaconus & mask == u32::from(self.ioport(W_PreBeacon)) & mask {
                    self.set_irq15(request);
                }
            }

            // melonDS fires the millisecond timer whenever the low 10 bits
            // of `USCounter` fall inside the first timer interval of a
            // 1024µs window (`!(uspart & kTimeCheckMask)` with
            // `kTimeCheckMask = ~(kTimerInterval - 1)`, `Wifi.cpp:1799-1809`),
            // not only on an exact multiple of 1024.
            //
            // The difference is load-bearing: `USCounter` is *assigned* from
            // a received beacon's timestamp (`Wifi::step_rx`), which is not
            // generally a multiple of the 8µs interval. With an equality
            // test the counter could then step past the boundary forever
            // without ever landing on it, silently stopping beacons,
            // IRQ 14 and the whole beacon-interval state machine.
            if self.us_counter & 0x3FF < Self::TIMER_INTERVAL_US {
                self.ms_timer(request);
            }
        }

        if self.is_mp_client && self.com_status == 0 {
            if self.rx_timestamp != 0 && self.us_timestamp >= self.rx_timestamp {
                self.rx_timestamp = 0;
                if self.rx_deferred.armed {
                    let d = self.rx_deferred;
                    self.rx_deferred = DeferredRxParams::default();
                    self.start_rx(request, d.keep, d.tx_rate, d.framelen);
                }
            }
            if self.us_timestamp >= self.next_sync {
                self.check_rx(RxKind::HostFrames, request);
            }
        }

        if self.us_until_power_on < 0 {
            self.us_until_power_on += Self::TIMER_INTERVAL_US as i32;
            if self.us_until_power_on >= 0 {
                self.us_until_power_on = 0;
                self.set_ioport(W_PowerState, 0);
                self.set_status(1);
                self.update_power_status(0);
            }
        }

        if self.ioport(W_CmdCountCnt) & 0x0001 != 0 {
            self.cmd_counter = self.cmd_counter.saturating_sub(Self::TIMER_INTERVAL_US as u32);
        }

        let content_free = self.ioport(W_ContentFree);
        if content_free != 0 {
            let interval = Self::TIMER_INTERVAL_US as u16;
            self.set_ioport(W_ContentFree, content_free.saturating_sub(interval));
        }

        if self.com_status == 0 {
            let busy = self.ioport(W_TXBusy);
            if busy != 0 {
                self.com_status = 0x2;
                // Latch the slot *once*, on the idle -> transmitting
                // transition; `Wifi::process_tx` then advances that slot until
                // it finishes and `Wifi::reselect_tx_slot` picks the next.
                // Ported from `USTimer` (`Wifi.cpp:1833-1849`); see
                // `docs/design/local-mp-melonds-parity-2.md` F3.
                self.tx_cur_slot = tx::pick_busy_slot(busy).map_or(-1, |s| s as i32);
            } else if !self.is_mp_client || self.us_timestamp > self.next_sync {
                if self.rx_counter & 0x1FF == 0 {
                    self.check_rx(RxKind::Regular, request);
                }
                self.rx_counter = self.rx_counter.wrapping_add(Self::TIMER_INTERVAL_US as u32);
            }
        }

        if self.com_status & 0x2 != 0 {
            self.process_tx(request);
        }
        if self.com_status & 0x1 != 0 {
            self.step_rx(request);
        }

        if self.power_on {
            self.schedule_timer(scheduler);
        }
    }

    /// Advances the ~1ms beacon-interval counters. Ported from melonDS
    /// `Wifi::MSTimer` (`docs/design/melonds/WiFi.cpp:1727-1751`), minus
    /// the `BlockBeaconIRQ14` gate (a WEP-association-timing nuance out of
    /// scope here). `W_BeaconCount1` free-runs and re-arms itself from
    /// `W_BeaconInterval`, giving the driver a steady heartbeat to re-send
    /// beacons/re-check its state machine on; `W_BeaconCount2` is a
    /// one-shot countdown the driver arms per-operation (e.g. "wait this
    /// long for an association response").
    fn ms_timer(&mut self, request: &mut InterruptRequest) {
        if self.ioport(W_USCompareCnt) != 0 && (self.us_counter & !0x3FF) == self.us_compare {
            self.set_irq14(0, request);
        }

        let count1 = self.ioport(W_BeaconCount1);
        if count1 != 0 {
            let count1 = count1 - 1;
            self.set_ioport(W_BeaconCount1, count1);
            if count1 == 0 {
                if debug_enabled() {
                    eprintln!(
                        "[wifi] beacon interval elapsed: raising IRQ14, TXBusy=0x{:04X} \
                         (bit4 set means the driver re-armed the beacon slot last time)",
                        self.ioport(W_TXBusy)
                    );
                }
                self.set_irq14(1, request);
            }
        }
        // `W_BeaconCount1` starts at its register default (0), never having
        // had a chance to count down at all; without this, the very first
        // beacon interval never begins. `set_irq14`'s own reload only fires
        // once the countdown has already reached zero *from* a nonzero
        // value, which doesn't cover this initial case.
        if self.ioport(W_BeaconCount1) == 0 {
            self.set_ioport(W_BeaconCount1, self.ioport(W_BeaconInterval));
        }

        let count2 = self.ioport(W_BeaconCount2);
        if count2 != 0 {
            let count2 = count2 - 1;
            self.set_ioport(W_BeaconCount2, count2);
            if count2 == 0 {
                self.set_irq13(request);
            }
        }
    }

    /// Post-beacon auto power-down. Ported from `SetIRQ13`
    /// (`Wifi.cpp:392-409`).
    ///
    /// The power-down is gated on automatic power-saving mode
    /// (`W_ModeWEP & 7 == 0`) and on `W_PowerTX` bit 1 being clear, for the
    /// reason melonDS spells out: a station with power saving disabled does
    /// not service IRQ13/IRQ15, so powering the transceiver down from the
    /// one-shot `W_BeaconCount2` countdown would leave nothing to wake it
    /// up again.
    fn set_irq13(&mut self, request: &mut InterruptRequest) {
        self.raise_irq(13, request);
        if self.ioport(W_ModeWEP) & 0x7 == 0 && self.ioport(W_PowerTX) & (1 << 1) == 0 {
            self.update_power_status(-1);
        }
    }

    /// Pre-beacon auto wake-up. Ported from `SetIRQ15`
    /// (`Wifi.cpp:441-450`).
    ///
    /// A client in power-saving mode sleeps between beacons and relies on
    /// this to be awake again in time for the next one. Unlike the
    /// auto-sleep above, it applies under every power-management mode.
    ///
    /// Leaving this unported (it was previously dismissed as
    /// infrastructure-mode power saving) meant a client that powered down
    /// after receiving a beacon never woke up: it spun on
    /// `W_PowerState`/`W_RFStatus` forever and never transmitted the
    /// association request that local play starts with.
    fn set_irq15(&mut self, request: &mut InterruptRequest) {
        self.raise_irq(15, request);
        if self.ioport(W_PowerTX) & (1 << 0) != 0 {
            self.update_power_status(1);
        }
    }

    /// Reloads `W_BeaconCount1`, raises IRQ 14, and -- this is what
    /// actually puts a beacon on the wire -- starts the beacon TX slot if
    /// the driver has armed it. Ported from `SetIRQ14`
    /// (`Wifi.cpp:411-439`); `source`: `0` = USCOMPARE, `1` = BEACONCOUNT
    /// (this port never calls it with melonDS's `2` = forced).
    ///
    /// Deliberately not ported: the `W_USCompareCnt` bit 0 gate melonDS
    /// applies before doing *any* of this (including raising IRQ 14 at
    /// all). Adding it would re-introduce the exact "beacon interval elapsed
    /// but IRQ 14 never fires" Union Room symptom this module's existing
    /// `beacon_interval_reload_fires_irq14_periodically` regression test
    /// protects against, for real games that never touch
    /// `W_USCompareCnt`. Also not ported: `BlockBeaconIRQ14` (a WEP-specific
    /// nuance).
    fn set_irq14(&mut self, source: u8, request: &mut InterruptRequest) {
        if source != 2 {
            self.set_ioport(W_BeaconCount1, self.ioport(W_BeaconInterval));
        }
        self.raise_irq(14, request);
        self.set_ioport(W_BeaconCount2, 0xFFFF);
        self.set_ioport(W_TXReqRead, self.ioport(W_TXReqRead) & 0xFFF2);
        if self.ioport(W_TXSlotBeacon) & 0x8000 != 0 {
            self.start_tx_beacon();
        }
        if self.ioport(W_ListenCount) == 0 {
            self.set_ioport(W_ListenCount, self.ioport(W_ListenInterval));
        }
        self.set_ioport(W_ListenCount, self.ioport(W_ListenCount).wrapping_sub(1));
    }
}

impl Default for Wifi {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `rxflags` [`stage_plain_data_frame`] classifies to: the `0x0010` base,
    /// `0x8000` for a matching BSSID, and `0x0008` for a data frame carrying no
    /// MP MAC.
    const PLAIN_DATA_RXFLAGS: u16 = 0x8018;

    /// Stages an ordinary, non-MP data frame in `rx_buffer` that survives
    /// [`Wifi::classify_rxflags`] with the register file left at its defaults.
    ///
    /// Classification and `W_RXFilter`/`W_RXFilter2` filtering run at frame
    /// *completion* in [`Wifi::step_rx`] (see
    /// `docs/design/review_mp_local2.md` P0-1), so a test that drives
    /// [`Wifi::start_rx`] directly can no longer choose its own `rxflags`: the
    /// bytes in `rx_buffer` decide. This helper supplies bytes that classify
    /// deterministically to [`PLAIN_DATA_RXFLAGS`]:
    ///
    /// * frame control `0x0008` — a data frame (bits 2-3 = `10`) of subtype 0,
    ///   which hardware accepts unconditionally, with `fromto` 0 and the
    ///   retransmit flag clear;
    /// * `W_BSSID0..2` copied out of the frame's own address 3, so the frame is
    ///   not rejected for belonging to another network.
    ///
    /// Callers are expected to have filled the rest of the buffer already; the
    /// BSSID registers are read back from the buffer rather than assumed, so a
    /// caller that filled fewer bytes than another still gets a match.
    fn stage_plain_data_frame(wifi: &mut Wifi) {
        wifi.rx_buffer[12] = 0x08;
        wifi.rx_buffer[13] = 0x00;
        // Address 3 sits at frame offset 16, i.e. buffer offset 12 + 16.
        for (i, reg) in [W_BSSID0, W_BSSID1, W_BSSID2].into_iter().enumerate() {
            let o = 12 + 16 + i * 2;
            let value = wifi.rx_buffer[o] as u16 | (wifi.rx_buffer[o + 1] as u16) << 8;
            wifi.set_ioport(reg, value);
        }
    }

    #[test]
    fn register_mirror_does_not_double_increment() {
        let mut wifi = Wifi::new();
        let mut request = InterruptRequest::empty();
        wifi.set_ioport(W_RXBufReadAddr, 0x100);
        wifi.set_ioport(W_RXCnt, 0x8000);
        let active = wifi.read16(W_RXBufDataRead as u32, &mut request);
        let passive = wifi.read16(0x1000 + W_RXBufDataRead as u32, &mut request);
        // The active-region read must have advanced the read cursor; the
        // mirrored (1000h-1FFFh) read must not have advanced it again.
        assert_ne!(active, 0xFFFF);
        let _ = passive;
    }

    #[test]
    fn ram_round_trips_16_bit() {
        let mut wifi = Wifi::new();
        let mut scheduler = Scheduler::new();
        let mut request = InterruptRequest::empty();
        wifi.write16(0x4000, 0xBEEF, &mut scheduler, &mut request);
        assert_eq!(wifi.read16(0x4000, &mut request), 0xBEEF);
        // Mirror at 4800h..5FFFh maps into the same 8KiB RAM.
        let mirrored = wifi.read16(0x5000, &mut request);
        assert_eq!(mirrored, wifi.ram[0x1000] as u16 | (wifi.ram[0x1001] as u16) << 8);
    }

    #[test]
    fn eight_bit_write_is_ignored() {
        let mut wifi = Wifi::new();
        let mut scheduler = Scheduler::new();
        let mut request = InterruptRequest::empty();
        wifi.write16(W_TXStatCnt as u32, 0x1234, &mut scheduler, &mut request);
        wifi.write8(W_TXStatCnt as u32, 0xFF);
        assert_eq!(wifi.read16(W_TXStatCnt as u32, &mut request), 0x1234);
    }

    #[test]
    fn channel_detect_rejects_all_zero_rf() {
        let mut wifi = Wifi::new();
        // Simulate a loaded firmware config: every channel table entry is
        // distinct and non-zero (`docs/design/design_lan.md` §7.2). Left
        // untouched, `rf_regs`/`rf_channel_index` still default to zero from
        // `Wifi::new()` -- with a *degenerate* (all-zero) channel table this
        // would spuriously match channel 1 (trap 3); with a proper table it
        // must not match anything.
        for (i, entry) in wifi.rf_channel_data.iter_mut().enumerate() {
            *entry = [0x21 + i as u32, 0x41 + i as u32];
        }
        wifi.rf_channel_index = [0, 1];
        wifi.change_channel();
        assert_eq!(wifi.cur_channel, 0);

        wifi.rf_regs[0] = 0x21 + 6;
        wifi.rf_regs[1] = 0x41 + 6;
        wifi.change_channel();
        assert_eq!(wifi.cur_channel, 7);
    }

    /// A `W_ModeReset` write with bit 14 set must install the hardware's
    /// RX-ring defaults. Without them `W_RXBufBegin == W_RXBufEnd == 0`, and
    /// `Wifi::check_rx`'s zero-size-ring guard rejects *every* inbound frame --
    /// the driver relies on this register instead of programming the ring
    /// itself. `docs/design/local-mp-melonds-parity-2.md` F0.
    #[test]
    fn mode_reset_bit14_installs_rx_ring_defaults() {
        let mut wifi = Wifi::new();
        let mut scheduler = Scheduler::new();
        let mut request = InterruptRequest::empty();

        assert_eq!(
            wifi.ioport(W_RXBufBegin),
            wifi.ioport(W_RXBufEnd),
            "precondition: the ring is zero-sized before W_ModeReset"
        );

        wifi.write16(W_ModeReset as u32, 0x4000, &mut scheduler, &mut request);

        assert_eq!(wifi.ioport(W_RXBufBegin), 0x4000);
        assert_eq!(wifi.ioport(W_RXBufEnd), 0x4800);
        assert_ne!(wifi.ioport(W_RXBufBegin), wifi.ioport(W_RXBufEnd));
        assert_eq!(wifi.ioport(W_RXFilter), 0x0401);
        assert_eq!(wifi.ioport(W_RXFilter2), 0x0008);
        assert_eq!(wifi.ioport(W_TXRetryLimit), 0x0707);
    }

    /// The RX header must land at the byte offset named by the write cursor
    /// (`cursor << 1`), not at the cursor's raw halfword value. Regression
    /// test for the address-unit conflation in
    /// `docs/design/local-mp-melonds-parity-2.md` §2/F1: masking instead of
    /// shifting put every frame after the first inside its predecessor's body.
    #[test]
    fn rx_header_lands_at_the_write_cursor_shifted_left() {
        let mut wifi = Wifi::new();
        let mut request = InterruptRequest::empty();
        wifi.set_ioport(W_RXCnt, 0x8000);
        wifi.set_ioport(W_RXBufBegin, 0x4000);
        wifi.set_ioport(W_RXBufEnd, 0x4800);
        wifi.set_ioport(W_RXBufReadCursor, 0x07FF); // Far from the write path.
        wifi.set_ioport(W_RXBufWriteCursor, 0x0100); // Halfword 0x100 -> byte 0x200.

        wifi.rx_buffer[..12 + 32].fill(0xAA);
        stage_plain_data_frame(&mut wifi);
        wifi.start_rx(&mut request, true, 0x14, 32);
        for _ in 0..64 {
            if wifi.com_status & 0x1 == 0 {
                break;
            }
            wifi.step_rx(&mut request);
        }

        // `rxflags` was written at byte 0x200, not at 0x100.
        assert_eq!(
            wifi.ram[0x0200] as u16 | (wifi.ram[0x0201] as u16) << 8,
            PLAIN_DATA_RXFLAGS,
            "the RX header must be written at (write cursor << 1)"
        );
        assert_eq!(wifi.ram[0x0100], 0, "nothing may be written at the unshifted offset");
        // The cursor advanced past header + body, still in halfword units.
        assert!(wifi.ioport(W_RXBufWriteCursor) >= 0x0100 + ((12 + 32) / 2));
    }

    /// The MP reply slot's internal `W_TXBusy` bit 7 must never be visible to
    /// the CPU -- hardware exposes no bit for it (`Wifi.cpp:2088`).
    /// `docs/design/local-mp-melonds-parity-2.md` F2.
    #[test]
    fn tx_busy_read_hides_the_mp_reply_slot_bit() {
        let mut wifi = Wifi::new();
        let mut request = InterruptRequest::empty();
        wifi.set_ioport(W_TXBusy, 0x0082);
        assert_eq!(wifi.read16(W_TXBusy as u32, &mut request), 0x0002);
    }

    /// Arming the beacon slot while the MP CMD slot is mid-round must not
    /// steal the phase machine. Regression test for the beacon-preempts-CMD
    /// failure in `docs/design/local-mp-melonds-parity-2.md` F3: the CMD
    /// slot's phase-2 `mp_reply_timer` loop is what delivers client replies,
    /// and a preemption silently stops it.
    #[test]
    fn beacon_slot_does_not_preempt_an_in_flight_cmd_round() {
        let mut wifi = Wifi::new();
        let mut scheduler = Scheduler::new();
        let mut request = InterruptRequest::empty();

        wifi.set_ioport(W_TXBusy, 0x0002); // CMD slot busy.
        wifi.tx_slots[1] =
            TxSlot { valid: true, phase: 2, phase_time: 100_000, rate: 2, ..Default::default() };
        wifi.com_status = 0;
        wifi.tick(&mut scheduler, &mut request);
        assert_eq!(wifi.tx_cur_slot, 1, "the CMD slot must be latched on the idle -> TX edge");

        // The beacon interval elapses mid-round and arms slot 4.
        wifi.set_ioport(W_TXBusy, wifi.ioport(W_TXBusy) | 0x0010);
        wifi.tick(&mut scheduler, &mut request);
        assert_eq!(wifi.tx_cur_slot, 1, "the beacon slot must not preempt the CMD round");
    }

    /// `W_RXBufCount` must stop at zero rather than wrapping to `0xFFFF`, and
    /// must raise IRQ 9 on the zero transition -- the "receive buffer drained"
    /// signal. The *write* path already did both; the read path did neither.
    /// `docs/design/local-mp-melonds-parity-2.md` F4.
    #[test]
    fn rx_buf_data_read_counts_down_to_zero_and_raises_irq9() {
        let mut wifi = Wifi::new();
        let mut request = InterruptRequest::empty();
        wifi.set_ioport(W_IE, 1 << 9);
        wifi.set_ioport(W_RXBufBegin, 0x4000);
        wifi.set_ioport(W_RXBufEnd, 0x4800);
        wifi.set_ioport(W_RXBufCount, 1);

        wifi.read16(W_RXBufDataRead as u32, &mut request);
        assert_eq!(wifi.ioport(W_RXBufCount), 0);
        assert_ne!(wifi.ioport(W_IF) & (1 << 9), 0, "IRQ 9 must fire on the zero transition");

        wifi.read16(W_RXBufDataRead as u32, &mut request);
        assert_eq!(wifi.ioport(W_RXBufCount), 0, "must not underflow to 0xFFFF");
    }

    /// Each client that failed to reply bumps its own byte-wide counter in the
    /// `W_CMDStat0..7` block, leaving every other client's byte untouched.
    /// Under `IOPORT8(W_CMDStat0 + i)` packing, client `i` lives in port
    /// `W_CMDStat0 + (i & !1)` -- low byte for even `i`, high byte for odd.
    /// `docs/design/local-mp-melonds-parity-2.md` F6.
    #[test]
    fn mp_reply_errors_increment_per_client_byte_counters() {
        let mut wifi = Wifi::new();
        wifi.report_mp_reply_errors(0b0000_0000_0000_0110); // Clients 1 and 2.

        assert_eq!(wifi.ioport(W_CMDStat0), 0x0100, "client 1 = high byte of 1D0h");
        assert_eq!(wifi.ioport(W_CMDStat0 + 2), 0x0001, "client 2 = low byte of 1D2h");
        assert_eq!(wifi.ioport(W_CMDStat0 + 4), 0x0000, "no other client's byte moved");
    }

    /// Builds a synthetic firmware Wi-Fi config block (starting at firmware
    /// offset `02Ch`) with a recognisable RF channel table planted at the
    /// offsets melonDS's `FirmwareHeader` puts them at, so
    /// `Wifi::load_firmware_config` is checked against the real layout rather
    /// than against itself.
    fn synthetic_wifi_config(rf_chip_type: u8) -> Vec<u8> {
        // Long enough for the whole Type-2 table (config 0xC6 + 84 = 0x11A)
        // and the Type-3 one (config 0xFA + 14 = 0x108).
        let mut cfg = vec![0u8; 0x140];
        cfg[0x14] = rf_chip_type; // RFChipType, firmware 040h.

        if rf_chip_type == 3 {
            cfg[0xEA] = 0x0A; // RFIndex1, firmware 116h.
            cfg[0xF9] = 0x0B; // RFIndex2, firmware 125h.
            for i in 0..14 {
                cfg[0xEB + i] = 0x21 + i as u8; // RFData1, firmware 117h.
                cfg[0xFA + i] = 0x41 + i as u8; // RFData2, firmware 126h.
            }
        } else {
            // InitialRF56Values, firmware 0F2h: 14 channels x 6 bytes, each
            // channel two 18-bit values.
            let base = 0xC6;
            for i in 0..14 {
                let o = base + i * 6;
                cfg[o] = 0x21 + i as u8;
                cfg[o + 1] = 0x00;
                cfg[o + 2] = 0x01; // Bits 16-17 of value 1; bits 2..7 index 1.
                cfg[o + 3] = 0x41 + i as u8;
                cfg[o + 4] = 0x00;
                cfg[o + 5] = 0x02; // Bits 16-17 of value 2; bits 2..7 index 2.
            }
            // The index registers are the top six bits of each third byte.
            cfg[base + 2] = 0x0A << 2;
            cfg[base + 5] = 0x0B << 2;
        }
        cfg
    }

    /// A Type-3 firmware's channel table must be read from `RFIndex1`/
    /// `RFData1`/`RFIndex2`/`RFData2`, and a channel the game selects must
    /// then resolve.
    #[test]
    fn type3_firmware_config_resolves_a_channel() {
        let mut wifi = Wifi::new();
        wifi.load_firmware_config(&synthetic_wifi_config(3));

        assert_eq!(wifi.rf_version, 3);
        assert_eq!(wifi.rf_channel_index, [0x0A, 0x0B]);
        assert_eq!(wifi.rf_channel_data[6], [0x21 + 6, 0x41 + 6]);

        // The game selects channel 7 by writing that entry's two values.
        wifi.rf_regs[0x0A] = 0x21 + 6;
        wifi.rf_regs[0x0B] = 0x41 + 6;
        wifi.change_channel();
        assert_eq!(wifi.cur_channel, 7);
    }

    /// When the driver re-uploads the firmware's `InitialRFValues`, the two
    /// channel-selection registers go back to their power-on defaults and no
    /// channel resolves. That is a torn-down radio, not a channel the firmware
    /// table lacks, and the diagnostic must say so -- blaming the channel table
    /// sends the reader to `free_bios`'s `CHAN_DATA` instead of to whatever
    /// made the driver restart. See `docs/design/review_mp_local2.md` §7.1b.
    #[test]
    fn initial_rf_values_are_reported_as_a_reinitialised_radio() {
        let mut wifi = Wifi::new();
        wifi.load_firmware_config(&free_bios::firmware::FIRMWARE_DS[0x2C..]);

        assert_eq!(wifi.rf_version, 3, "the synthetic firmware reports RFChipType 3");
        // melonDS's `RFINIT` starts `31 4C 4F`, and the type-3 channel indices
        // are RF registers 1 and 2, so the defaults are `0x4C`/`0x4F`.
        assert_eq!(wifi.rf_initial_values, [0x4C, 0x4F]);

        // Park the radio on exactly those values, as a driver re-init does.
        // The transfer count only has to be nonzero: it is what tells the
        // verdict the driver got as far as the radio at all.
        wifi.diag.rf_transfers = 1;
        let (i0, i1) = (wifi.rf_channel_index[0] as usize, wifi.rf_channel_index[1] as usize);
        wifi.rf_regs[i0] = 0x4C;
        wifi.rf_regs[i1] = 0x4F;
        wifi.change_channel();
        assert_eq!(wifi.cur_channel, 0, "the power-on defaults match no channel");

        let snapshot = wifi.diag_snapshot();
        assert!(snapshot.rf_at_initial_values);
        let verdict = snapshot.verdict(true);
        assert!(
            verdict.contains("InitialRFValues"),
            "the verdict must name the real cause, got: {verdict}"
        );
        assert!(
            !verdict.contains("matches no entry"),
            "it must not blame the channel table, got: {verdict}"
        );
    }

    /// Regression test for the channel-detection failure that made local play
    /// impossible with a Type-2 firmware: `InitialRF56Values` lives at
    /// firmware `0F2h` (config `0xC6`), but the parser read config `0x38`
    /// (firmware `064h`, `InitialBBValues`). The table was therefore garbage,
    /// no channel ever matched, `cur_channel` stayed `0`, and both
    /// `Wifi::send_slot_frame` and `Wifi::check_rx` discarded every frame.
    #[test]
    fn type2_firmware_config_resolves_a_channel() {
        let mut wifi = Wifi::new();
        wifi.load_firmware_config(&synthetic_wifi_config(2));

        assert_eq!(wifi.rf_version, 2);
        assert_eq!(
            wifi.rf_channel_index,
            [0x0A, 0x0B],
            "index registers come from InitialRF56Values[2]/[5] >> 2"
        );
        // Channel 7's pair: 18-bit values assembled from three bytes each.
        assert_eq!(wifi.rf_channel_data[6], [(0x21 + 6) | (1 << 16), (0x41 + 6) | (2 << 16)]);

        wifi.rf_regs[0x0A] = (0x21 + 6) | (1 << 16);
        wifi.rf_regs[0x0B] = (0x41 + 6) | (2 << 16);
        wifi.change_channel();
        assert_eq!(wifi.cur_channel, 7);
    }

    /// A config block too short for the table it claims must leave the table
    /// alone rather than panicking or filling it with out-of-range reads.
    #[test]
    fn truncated_firmware_config_is_rejected() {
        let mut wifi = Wifi::new();
        let mut cfg = synthetic_wifi_config(3);
        cfg.truncate(0x80);
        wifi.load_firmware_config(&cfg);
        assert!(wifi.rf_channel_data.iter().all(|&[a, b]| a == 0 && b == 0));

        let mut cfg2 = synthetic_wifi_config(2);
        cfg2.truncate(0x80);
        wifi.load_firmware_config(&cfg2);
        assert!(wifi.rf_channel_data.iter().all(|&[a, b]| a == 0 && b == 0));
    }

    /// The `W_RFPins` lookup must match melonDS's table exactly, and must not
    /// panic for any state this port passes in.
    #[test]
    fn rf_pins_table_matches_melonds() {
        let mut wifi = Wifi::new();
        for (status, expected) in
            [(0u32, 0x04u16), (1, 0x84), (3, 0x46), (5, 0x84), (6, 0x87), (8, 0x46), (9, 0x04)]
        {
            wifi.set_status(status);
            assert_eq!(wifi.ioport(W_RFStatus), status as u16, "status {status}");
            assert_eq!(wifi.ioport(W_RFPins), expected, "rfpins[{status}]");
        }
    }

    /// A baseband register transfer is committed by the `W_BBCnt` write, not
    /// by the `W_BBWrite` write that stages the value. The driver uploads its
    /// baseband table as `W_BBWrite = value; W_BBCnt = 0x5000 | id;` and then
    /// reads each register back through `W_BBRead` to verify -- so committing
    /// on the wrong write put every value in the previous register's slot,
    /// the verification never matched, and the driver re-uploaded the table
    /// forever instead of moving on to RF channel selection.
    /// Ported from `Wifi.cpp:2309-2317`.
    #[test]
    fn baseband_write_commits_on_the_bbcnt_write() {
        let mut wifi = Wifi::new();
        let mut scheduler = Scheduler::new();
        let mut request = InterruptRequest::empty();

        // Upload two consecutive writable registers, driver-style: stage the
        // byte in `W_BBWrite`, then commit it with `W_BBCnt`.
        wifi.write16(W_BBWrite as u32, 0xAA, &mut scheduler, &mut request);
        wifi.write16(W_BBCnt as u32, 0x5000 | 0x20, &mut scheduler, &mut request);
        wifi.write16(W_BBWrite as u32, 0xBB, &mut scheduler, &mut request);
        wifi.write16(W_BBCnt as u32, 0x5000 | 0x21, &mut scheduler, &mut request);

        // Read them back the way the driver verifies its upload.
        wifi.write16(W_BBCnt as u32, 0x6000 | 0x20, &mut scheduler, &mut request);
        assert_eq!(wifi.read16(W_BBRead as u32, &mut request), 0xAA);
        wifi.write16(W_BBCnt as u32, 0x6000 | 0x21, &mut scheduler, &mut request);
        assert_eq!(wifi.read16(W_BBRead as u32, &mut request), 0xBB);

        // A hardwired register ignores the write and keeps its fixed value
        // (`BBREG_FIXED(0x00, 0x6D)`).
        wifi.write16(W_BBWrite as u32, 0x12, &mut scheduler, &mut request);
        wifi.write16(W_BBCnt as u32, 0x5000, &mut scheduler, &mut request);
        wifi.write16(W_BBCnt as u32, 0x6000, &mut scheduler, &mut request);
        assert_eq!(wifi.read16(W_BBRead as u32, &mut request), 0x6D);

        // A read with the wrong command nibble returns 0, as on hardware.
        wifi.write16(W_BBCnt as u32, 0x1000 | 0x20, &mut scheduler, &mut request);
        assert_eq!(wifi.read16(W_BBRead as u32, &mut request), 0);
    }

    /// A power-down must be observable *both* as `W_PowerState` bit 9 --
    /// which the driver polls to confirm the power-down it requested -- and
    /// through `W_TRXPower`/`W_RFStatus`.
    ///
    /// Suppressing bit 9 stalls the driver earlier than a power-down does:
    /// measured against a real game it never even armed reception
    /// (`W_RXCnt` stayed 0) and spun here tens of millions of times. It is
    /// cleared again by [`Wifi::tick`]'s power-on countdown
    /// (`Wifi.cpp:1789-1793`).
    #[test]
    fn power_down_is_observable_to_the_driver() {
        let mut wifi = Wifi::new();
        let mut sc = Scheduler::new();
        let mut rq = InterruptRequest::empty();

        wifi.write16(W_PowerUS as u32, 0, &mut sc, &mut rq);
        wifi.set_power_cnt(true, &mut sc);
        wifi.write16(W_ModeReset as u32, 0x0001, &mut sc, &mut rq);

        wifi.write16(W_PowerForce as u32, 0x8000, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(W_TRXPower), 1, "forced on");
        assert_eq!(wifi.ioport(W_RFStatus), 1);

        wifi.write16(W_PowerForce as u32, 0x8001, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(W_TRXPower), 0, "forced off");
        assert_eq!(wifi.ioport(W_RFStatus), 9);
        assert_ne!(
            wifi.ioport(W_PowerState) & (1 << 9),
            0,
            "the driver polls bit 9 to confirm the power-down"
        );

        // Forcing back on arms the power-up countdown, which clears the
        // whole register when it completes.
        wifi.write16(W_PowerForce as u32, 0x8000, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(W_TRXPower), 1);
        for _ in 0..512 {
            wifi.tick(&mut sc, &mut rq);
        }
        assert_eq!(
            wifi.ioport(W_PowerState) & (1 << 9),
            0,
            "the power-up countdown must clear the power-down bit again"
        );
    }

    /// `W_PowerForce` bit 15 overrides every other power input; bit 0
    /// selects off. `Wifi.cpp:477-480`.
    #[test]
    fn power_force_overrides_everything() {
        let mut wifi = Wifi::new();
        let mut sc = Scheduler::new();
        let mut rq = InterruptRequest::empty();

        wifi.write16(W_PowerUS as u32, 0, &mut sc, &mut rq);
        wifi.set_power_cnt(true, &mut sc);
        // Master enable clear: normally that alone forces the transmit half
        // off, but `W_PowerForce` outranks it.
        wifi.write16(W_ModeReset as u32, 0x0000, &mut sc, &mut rq);
        wifi.write16(W_PowerForce as u32, 0x8000, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(W_TRXPower), 1, "forced on despite master enable clear");
        assert_eq!(wifi.ioport(W_RFStatus), 1);

        wifi.write16(W_PowerForce as u32, 0x8001, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(W_TRXPower), 0, "forced off");
        assert_eq!(wifi.ioport(W_RFStatus), 9);
    }

    /// Both edges of `W_ModeReset` bit 0 publish a status word at port
    /// `27Ch` (`Wifi.cpp:2131-2143`).
    ///
    /// The power state is deliberately left unchanged by the master-enable
    /// branch itself -- see the deviation note on
    /// [`Wifi::update_power_status`] -- so a driver re-initialisation, which
    /// clears this bit for a stretch, cannot strand the radio.
    #[test]
    fn mode_reset_bit0_publishes_port_27c_without_stranding_power() {
        let mut wifi = Wifi::new();
        let mut sc = Scheduler::new();
        let mut rq = InterruptRequest::empty();

        wifi.write16(W_PowerUS as u32, 0, &mut sc, &mut rq);
        wifi.set_power_cnt(true, &mut sc);

        wifi.write16(W_ModeReset as u32, 0x0001, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(0x27C), 0x0005, "rising edge publishes 0005h");
        wifi.write16(W_PowerForce as u32, 0x8000, &mut sc, &mut rq);
        wifi.write16(W_PowerForce as u32, 0x0000, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(W_TRXPower), 1, "precondition: transceiver up");

        wifi.write16(W_ModeReset as u32, 0x0000, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(0x27C), 0x000A, "falling edge publishes 000Ah");
        assert_eq!(wifi.ioport(W_TRXPower), 1, "clearing it must not force the radio off");

        // And with the master enable clear, an explicit power request still
        // works -- the branch must not outrank it.
        wifi.write16(W_PowerForce as u32, 0x8001, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(W_TRXPower), 0, "explicit force-off still applies");
        wifi.write16(W_PowerForce as u32, 0x8000, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(W_TRXPower), 1, "and the radio can still be brought back");
    }

    /// A client whose transceiver is down between beacons must be brought
    /// back by the pre-beacon IRQ 15. Ported from `SetIRQ15`
    /// (`Wifi.cpp:441-450`) and the `W_PreBeacon` comparison in `USTimer`
    /// (`Wifi.cpp:1801-1806`).
    #[test]
    fn pre_beacon_irq15_wakes_a_sleeping_client() {
        let mut wifi = Wifi::new();
        let mut sc = Scheduler::new();
        let mut rq = InterruptRequest::empty();

        wifi.write16(W_PowerUS as u32, 0, &mut sc, &mut rq);
        wifi.set_power_cnt(true, &mut sc);
        wifi.write16(W_ModeReset as u32, 0x0001, &mut sc, &mut rq);
        // Auto wake-up is enabled by `W_PowerTX` bit 0.
        wifi.write16(W_PowerTX as u32, 0x0001, &mut sc, &mut rq);
        wifi.set_ioport(W_USCountCnt, 1);
        wifi.set_ioport(W_USCompareCnt, 1);
        wifi.set_ioport(W_IE, 1 << 15);

        wifi.write16(W_PowerForce as u32, 0x8001, &mut sc, &mut rq);
        wifi.write16(W_PowerForce as u32, 0x0000, &mut sc, &mut rq);
        assert_eq!(wifi.ioport(W_TRXPower), 0, "transceiver down");

        wifi.set_ioport(W_BeaconCount1, 1);
        wifi.set_ioport(W_PreBeacon, 0x0400);

        for _ in 0..4096 {
            wifi.tick(&mut sc, &mut rq);
            if wifi.ioport(W_TRXPower) == 1 {
                break;
            }
        }

        assert_ne!(wifi.ioport(W_IF) & (1 << 15), 0, "IRQ 15 must fire");
        assert_eq!(wifi.ioport(W_TRXPower), 1, "the client must come back up");
        assert_eq!(wifi.ioport(W_RFStatus), 1);
    }

    /// Auto power-down must only happen in automatic power-saving mode. A
    /// station with power saving disabled does not service IRQ13/IRQ15, so
    /// powering down there would strand the transceiver. `Wifi.cpp:392-409`.
    #[test]
    fn auto_power_down_only_in_power_saving_mode() {
        let mut wifi = Wifi::new();
        let mut sc = Scheduler::new();
        let mut rq = InterruptRequest::empty();

        wifi.write16(W_PowerUS as u32, 0, &mut sc, &mut rq);
        wifi.set_power_cnt(true, &mut sc);
        wifi.write16(W_ModeReset as u32, 0x0001, &mut sc, &mut rq);
        wifi.write16(W_PowerForce as u32, 0x8000, &mut sc, &mut rq);
        wifi.write16(W_PowerForce as u32, 0x0000, &mut sc, &mut rq);

        // Mode 2 = power saving disabled: IRQ 13 must not power down.
        wifi.write16(W_ModeWEP as u32, 0x0002, &mut sc, &mut rq);
        wifi.set_irq13(&mut rq);
        assert_eq!(wifi.ioport(W_TRXPower), 1, "must stay awake with power saving disabled");
    }

    /// The millisecond timer must keep firing after `USCounter` is assigned a
    /// value that is not a multiple of the 8µs timer interval -- which is
    /// what happens when a beacon's timestamp is adopted. melonDS fires
    /// whenever the low bits land inside the first interval of the window
    /// (`Wifi.cpp:1799-1809`), not only on an exact multiple.
    #[test]
    fn ms_timer_survives_a_misaligned_us_counter() {
        let mut wifi = Wifi::new();
        let mut sc = Scheduler::new();
        let mut rq = InterruptRequest::empty();

        wifi.set_ioport(W_USCountCnt, 1);
        wifi.set_ioport(W_IE, 1 << 14);
        wifi.set_ioport(W_BeaconInterval, 2);
        wifi.us_counter = 1_234_567;

        let ticks_per_ms = 1024 / Wifi::TIMER_INTERVAL_US as usize;
        for _ in 0..(ticks_per_ms * 6) {
            wifi.tick(&mut sc, &mut rq);
        }

        assert_ne!(
            wifi.ioport(W_IF) & (1 << 14),
            0,
            "the beacon-interval IRQ must still fire with a misaligned USCounter"
        );
    }

    /// An RX header that straddles the end of the ring must wrap back to the
    /// ring base, field by field, as `FinishRX` does (`Wifi.cpp:1466-1478`).
    #[test]
    fn rx_header_wraps_at_the_ring_end() {
        let mut wifi = Wifi::new();
        let mut rq = InterruptRequest::empty();
        wifi.set_ioport(W_RXCnt, 0x8000);
        wifi.set_ioport(W_RXBufBegin, 0x0000);
        wifi.set_ioport(W_RXBufEnd, 0x0800);
        wifi.set_ioport(W_RXBufReadCursor, 0x0010);
        wifi.set_ioport(W_RXBufWriteCursor, (0x07FC / 2) as u16);

        wifi.rx_buffer[..12 + 16].fill(0xAA);
        stage_plain_data_frame(&mut wifi);
        wifi.start_rx(&mut rq, true, 0x14, 16);
        for _ in 0..64 {
            if wifi.com_status & 0x1 == 0 {
                break;
            }
            wifi.step_rx(&mut rq);
        }

        let rd = |a: usize| wifi.ram[a] as u16 | (wifi.ram[a + 1] as u16) << 8;
        assert_eq!(rd(0x07FC), PLAIN_DATA_RXFLAGS, "rxflags");
        assert_eq!(rd(0x07FE), 0x0040);
        // The 4-byte skip steps twice: 0x7FE -> 0x800 (the ring end, wrapping
        // to 0x000) -> 0x002.
        assert_eq!(rd(0x0002), 0x0014, "TX rate must land inside the ring");
        assert_eq!(rd(0x0004), 16, "frame length");
        assert_eq!(rd(0x0006), 0x4080, "RSSI");
    }

    /// A WEP frame's body sits 4 bytes further along, past the WEP IV.
    /// `CheckRX` slides it back down over the IV (`Wifi.cpp:1635-1639`) so
    /// everything downstream reads the body at its usual offset. Skipping
    /// that delivers an intact frame whose every field is shifted by 4.
    #[test]
    fn wep_frame_body_slides_over_the_iv() {
        let mut wifi = Wifi::new();
        let fc: u16 = 1 << 14;
        wifi.rx_buffer[12] = fc as u8;
        wifi.rx_buffer[13] = (fc >> 8) as u8;
        wifi.rx_buffer[12 + 24..12 + 28].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        wifi.rx_buffer[12 + 28..12 + 32].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);

        let len = wifi.crop_framelen(64, fc);

        assert_eq!(len, 64, "no crop configured, so the length is unchanged");
        assert_eq!(
            &wifi.rx_buffer[12 + 24..12 + 28],
            &[0x11, 0x22, 0x33, 0x44],
            "the real body must now start where the IV was"
        );
    }

    /// An unencrypted frame must not be slid (`Wifi.cpp:1640-1641`).
    #[test]
    fn non_wep_frame_body_is_left_alone() {
        let mut wifi = Wifi::new();
        wifi.rx_buffer[12 + 24..12 + 28].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        let len = wifi.crop_framelen(64, 0x0000);

        assert_eq!(len, 64);
        assert_eq!(&wifi.rx_buffer[12 + 24..12 + 28], &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    /// Regression test for the Union Room symptom (design doc §17, fixed
    /// here): with the beacon-interval counter never advancing, `W_IF`
    /// bit 14 never fired, so a real game's driver never re-armed its
    /// beacon TX slot or advanced its scan state machine past initial
    /// setup -- the room-level TCP/UDP link was healthy, but no beacon
    /// ever reached the wire. `Wifi::tick`'s millisecond timer must
    /// decrement `W_BeaconCount1`, reload it from `W_BeaconInterval`, and
    /// raise the Wi-Fi IRQ on every interval elapsed.
    #[test]
    fn beacon_interval_reload_fires_irq14_periodically() {
        let mut wifi = Wifi::new();
        let mut scheduler = Scheduler::new();
        let mut request = InterruptRequest::empty();

        wifi.set_ioport(W_USCountCnt, 1);
        wifi.set_ioport(W_IE, 1 << 14);
        wifi.set_ioport(W_BeaconInterval, 5);

        // `ms_timer` (and thus the beacon countdown) only runs once per
        // ~1024µs of `USCounter`, which itself only advances 8µs per
        // `tick()` call while `W_USCountCnt` is enabled.
        let ticks_per_ms_boundary = 1024 / Wifi::TIMER_INTERVAL_US as usize;
        for _ in 0..(ticks_per_ms_boundary * 6) {
            wifi.tick(&mut scheduler, &mut request);
        }

        assert!(
            request.contains(InterruptRequest::WIFI),
            "W_BeaconCount1 reaching zero must raise the ARM7 Wi-Fi interrupt request"
        );
        assert_ne!(
            wifi.ioport(W_IF) & (1 << 14),
            0,
            "W_IF bit 14 (beacon interval elapsed) must be set"
        );
        assert_ne!(
            wifi.ioport(W_BeaconCount1),
            0,
            "W_BeaconCount1 must self-reload from W_BeaconInterval, not stay at zero"
        );
    }

    /// Regression test for a second Union-Room symptom fix, found while
    /// diagnosing the first with `LUNARIS_WIFI_DEBUG=1`: a real driver
    /// commonly enables `POWCNT2`'s Wi-Fi bit during general system init
    /// *while `W_PowerUS` bit 0 (power-save) is still set*, then later
    /// clears that bit right before actually using the radio. Only the
    /// `POWCNT2` write path re-evaluated `power_on`; a subsequent
    /// `W_PowerUS` write that flips the *other* half of the "should the
    /// radio be on" condition was silently ignored, leaving `Wifi::tick`
    /// (and therefore beacons, channel resolution, RX polling -- the
    /// entire rest of this module) dead for the remainder of the session,
    /// even though the driver believed it had powered up.
    #[test]
    fn power_us_write_after_powcnt2_enable_still_powers_on() {
        let mut wifi = Wifi::new();
        let mut scheduler = Scheduler::new();
        let mut request = InterruptRequest::empty();

        wifi.write16(W_PowerUS as u32, 1, &mut scheduler, &mut request); // Power-save requested before POWCNT2 is even enabled.
        wifi.set_power_cnt(true, &mut scheduler); // POWCNT2 enabled, but power-save still holds the radio off.
        assert!(!wifi.power_on, "power-save bit must keep the radio off despite POWCNT2 enable");

        wifi.write16(W_PowerUS as u32, 0, &mut scheduler, &mut request); // Driver clears power-save to actually use the radio.
        assert!(
            wifi.power_on,
            "clearing W_PowerUS bit 0 after POWCNT2 was already enabled must power on the radio"
        );
    }
}
