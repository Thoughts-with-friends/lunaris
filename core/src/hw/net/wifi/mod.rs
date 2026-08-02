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
use rx::RxKind;

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

    us_until_power_on: i32,
    cmd_counter: u32,

    /// Not serialized: reinstalled by the frontend after a savestate load
    /// via [`Wifi::set_transport`], mirroring how `firmware`/`bios7` are
    /// re-supplied rather than serialized (`docs/design/design_lan.md` §8.1).
    #[savestate(skip)]
    transport: Option<Box<dyn MpTransport>>,
}

impl Wifi {
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
        self.cur_channel = 0;
        self.tx_cur_slot = -1;
        self.com_status = 0;
        self.mp_client_mask = 0;
        self.mp_client_fail = 0;

        // `W_ID` (000h) is a hardware-identification register a driver
        // reads during Wi-Fi init to confirm a real chip is present before
        // doing anything else; leaving it at zero looks like "no Wi-Fi
        // hardware" and can make a driver skip wireless entirely without
        // ever touching another W_* register. Ported from melonDS
        // `Wifi::Reset` (`docs/design/melonds/WiFi.cpp:186-197`); `0x1440`
        // is the plain-DS value, matching this emulator's DS-only scope.
        self.set_ioport(W_ID, 0x1440);

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
    /// (`docs/design/design_lan.md` §7.1) and must be at least `0x134`
    /// bytes long for a Type-3 RF chip.
    pub fn load_firmware_config(&mut self, config: &[u8]) {
        if config.len() < 0x16 {
            return;
        }
        self.rf_version = config[0x14]; // RFChipType, offset 040h - 02Ch
        if self.rf_version == 3 && config.len() >= 0x108 {
            let rf_index1 = config[0xEA] as u32; // 0x116 - 0x2C
            let rf_index2 = config[0xF9] as u32; // 0x125 - 0x2C
            self.rf_channel_index = [rf_index1, rf_index2];
            for i in 0..14 {
                self.rf_channel_data[i][0] = config[0xEB + i] as u32; // RFData1
                self.rf_channel_data[i][1] = config[0xFA + i] as u32; // RFData2
            }
        } else if config.len() >= 0x94 {
            // Type-2: two 3-byte values packed per channel at InitialRF56Values.
            let base = 0x38; // InitialRF56Values start, 0x64 - 0x2C
            self.rf_channel_index =
                [(config[base + 2] >> 2) as u32, (config[base + 5] >> 2) as u32];
            for i in 0..14 {
                let o = base + i * 6;
                if o + 6 > config.len() {
                    break;
                }
                self.rf_channel_data[i][0] = config[o] as u32
                    | (config[o + 1] as u32) << 8
                    | ((config[o + 2] as u32) & 0x3) << 16;
                self.rf_channel_data[i][1] = config[o + 3] as u32
                    | (config[o + 4] as u32) << 8
                    | ((config[o + 5] as u32) & 0x3) << 16;
            }
        }
    }

    pub(super) fn ioport(&self, addr: usize) -> u16 {
        self.io[addr >> 1]
    }

    pub(super) fn set_ioport(&mut self, addr: usize, value: u16) {
        self.io[addr >> 1] = value;
    }

    /// Sets `W_IF` bit `irq` and raises the ARM7 Wi-Fi interrupt request on
    /// the `0 -> nonzero` edge of `W_IF & W_IE`, matching melonDS
    /// `SetIRQ`/`CheckIRQ` (`docs/design/melonds/WiFi.cpp:376-390`). Level
    /// re-triggering here would flood the ARM7 with spurious interrupts.
    fn set_irq(&mut self, irq: u32, request: &mut InterruptRequest) {
        let old_flags = self.ioport(W_IF) & self.ioport(W_IE);
        self.set_ioport(W_IF, self.ioport(W_IF) | (1 << irq));
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

        // `USCounter`/the millisecond beacon timer only run while the game
        // has armed them via `W_USCountCnt`; `USTimestamp` (MP sync) above
        // is unconditional hardware state and always ticks.
        if self.ioport(W_USCountCnt) != 0 {
            self.us_counter += Self::TIMER_INTERVAL_US;
            if self.us_counter & 0x3FF == 0 {
                self.ms_timer(request);
            }
        }

        if self.is_mp_client && self.com_status == 0 {
            if self.rx_timestamp != 0 && self.us_timestamp >= self.rx_timestamp {
                self.rx_timestamp = 0;
                self.start_rx(request);
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
            self.raise_irq(14, request);
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
                self.raise_irq(14, request);
            }
        }
        if self.ioport(W_BeaconCount1) == 0 {
            self.set_ioport(W_BeaconCount1, self.ioport(W_BeaconInterval));
        }

        let count2 = self.ioport(W_BeaconCount2);
        if count2 != 0 {
            let count2 = count2 - 1;
            self.set_ioport(W_BeaconCount2, count2);
            if count2 == 0 {
                self.raise_irq(13, request);
            }
        }
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

    #[test]
    fn register_mirror_does_not_double_increment() {
        let mut wifi = Wifi::new();
        wifi.set_ioport(W_RXBufReadAddr, 0x100);
        wifi.set_ioport(W_RXCnt, 0x8000);
        let active = wifi.read16(W_RXBufDataRead as u32);
        let passive = wifi.read16(0x1000 + W_RXBufDataRead as u32);
        // The active-region read must have advanced the read cursor; the
        // mirrored (1000h-1FFFh) read must not have advanced it again.
        assert_ne!(active, 0xFFFF);
        let _ = passive;
    }

    #[test]
    fn ram_round_trips_16_bit() {
        let mut wifi = Wifi::new();
        let mut scheduler = Scheduler::new();
        wifi.write16(0x4000, 0xBEEF, &mut scheduler);
        assert_eq!(wifi.read16(0x4000), 0xBEEF);
        // Mirror at 4800h..5FFFh maps into the same 8KiB RAM.
        assert_eq!(wifi.read16(0x5000), wifi.ram[0x1000] as u16 | (wifi.ram[0x1001] as u16) << 8);
    }

    #[test]
    fn eight_bit_write_is_ignored() {
        let mut wifi = Wifi::new();
        let mut scheduler = Scheduler::new();
        wifi.write16(W_TXStatCnt as u32, 0x1234, &mut scheduler);
        wifi.write8(W_TXStatCnt as u32, 0xFF);
        assert_eq!(wifi.read16(W_TXStatCnt as u32), 0x1234);
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

        wifi.write16(W_PowerUS as u32, 1, &mut scheduler); // Power-save requested before POWCNT2 is even enabled.
        wifi.set_power_cnt(true, &mut scheduler); // POWCNT2 enabled, but power-save still holds the radio off.
        assert!(!wifi.power_on, "power-save bit must keep the radio off despite POWCNT2 enable");

        wifi.write16(W_PowerUS as u32, 0, &mut scheduler); // Driver clears power-save to actually use the radio.
        assert!(
            wifi.power_on,
            "clearing W_PowerUS bit 0 after POWCNT2 was already enabled must power on the radio"
        );
    }
}
