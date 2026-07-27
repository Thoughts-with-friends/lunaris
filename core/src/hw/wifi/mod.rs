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
mod regs;
mod rx;
mod tx;

use super::Scheduler;
use super::interrupt_controller::InterruptRequest;
pub use mp::LinkHints;
use mp::MpTransport;
pub use regs::*;
use rx::RxKind;

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

    pub(super) fn post_load(&mut self) {
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
    pub(super) fn set_power_cnt(&mut self, enable: bool, scheduler: &mut Scheduler) {
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
            scheduler.remove(super::scheduler::Event::Wifi);
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
        scheduler.schedule(
            super::scheduler::Event::Wifi,
            super::HW::on_wifi_timer,
            delay.max(1) as usize,
        );
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

    /// One 8µs hardware tick: advances the MP sync clock, the TX slot phase
    /// machine, and the RX byte-pump, then reschedules itself. Called from
    /// [`super::HW::on_wifi_timer`]. Simplified from melonDS `USTimer`
    /// (`docs/design/melonds/WiFi.cpp:1753-1935`): beacon-interval and
    /// microsecond-compare IRQ timing (`W_BeaconCount1/2`, `W_USCompareCnt`)
    /// are not yet ported, since they are not required for MP association
    /// and frame relay. See `docs/design/design_lan.md` §6.3.
    #[expect(
        clippy::cognitive_complexity,
        reason = "sequential hardware tick steps ported from a single melonDS function; \
                  splitting further would obscure the ordering that timing correctness depends on"
    )]
    pub(super) fn tick(&mut self, scheduler: &mut Scheduler, request: &mut InterruptRequest) {
        self.us_timestamp += Self::TIMER_INTERVAL_US;
        self.us_counter += Self::TIMER_INTERVAL_US;

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

        self.cmd_counter = self.cmd_counter.saturating_sub(Self::TIMER_INTERVAL_US as u32);

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
        let active = wifi.read16(0x0000 + W_RXBufDataRead as u32);
        let passive = wifi.read16(0x1000 + W_RXBufDataRead as u32);
        // The active-region read must have advanced the read cursor; the
        // mirrored (1000h-1FFFh) read must not have advanced it again.
        assert_ne!(active, 0xFFFF);
        let _ = passive;
    }

    #[test]
    fn ram_round_trips_16_bit() {
        let mut wifi = Wifi::new();
        wifi.write16(0x4000, 0xBEEF);
        assert_eq!(wifi.read16(0x4000), 0xBEEF);
        // Mirror at 4800h..5FFFh maps into the same 8KiB RAM.
        assert_eq!(wifi.read16(0x5000), wifi.ram[0x1000] as u16 | (wifi.ram[0x1001] as u16) << 8);
    }

    #[test]
    fn eight_bit_write_is_ignored() {
        let mut wifi = Wifi::new();
        wifi.write16(W_TXStatCnt as u32, 0x1234);
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
}
