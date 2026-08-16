//! Wi-Fi register-file (`W_*`) constants and the 16-bit-native read/write
//! dispatch. Constants are transcribed verbatim from melonDS
//! `docs/design/melonds/WiFi.hpp:31-160`; do not renumber them.
//!
//! The DS Wi-Fi hardware is **16-bit-native**: registers have side effects
//! (auto-incrementing buffer cursors, triggering BB/RF transfers,
//! write-1-to-clear on `W_IF`) that must fire exactly once per logical
//! access. `core/src/hw/mem/arm7/io.rs` therefore dispatches 16/32-bit CPU
//! accesses directly to [`Wifi::read16`]/[`Wifi::write16`] instead of
//! composing two 8-bit accesses. See `docs/design/design_lan.md` §6.2.

// Register names mirror the hardware ("W_*") 1:1, including case, both here
// and wherever they're matched on below. The full register list is kept even
// though this MP-focused subset doesn't reference every one, since it's the
// authoritative address map future work (WEP, beacons, WFC) will draw on.
#![allow(non_upper_case_globals, dead_code)]

use super::Wifi;
use crate::hw::{Scheduler, interrupt_controller::InterruptRequest};

pub mod names {
    pub const W_ID: usize = 0x000;
    pub const W_ModeReset: usize = 0x004;
    pub const W_ModeWEP: usize = 0x006;
    pub const W_TXStatCnt: usize = 0x008;
    pub const W_IF: usize = 0x010;
    pub const W_IE: usize = 0x012;
    pub const W_MACAddr0: usize = 0x018;
    pub const W_MACAddr1: usize = 0x01A;
    pub const W_MACAddr2: usize = 0x01C;
    pub const W_BSSID0: usize = 0x020;
    pub const W_BSSID1: usize = 0x022;
    pub const W_BSSID2: usize = 0x024;
    pub const W_AIDLow: usize = 0x028;
    pub const W_AIDFull: usize = 0x02A;
    pub const W_TXRetryLimit: usize = 0x02C;
    pub const W_RXCnt: usize = 0x030;
    pub const W_WEPCnt: usize = 0x032;
    pub const W_TRXPower: usize = 0x034;
    pub const W_PowerUS: usize = 0x036;
    pub const W_PowerTX: usize = 0x038;
    pub const W_PowerState: usize = 0x03C;
    pub const W_PowerForce: usize = 0x040;
    pub const W_PowerDownCtrl: usize = 0x48;
    pub const W_Random: usize = 0x044;
    pub const W_RXBufBegin: usize = 0x050;
    pub const W_RXBufEnd: usize = 0x052;
    pub const W_RXBufWriteCursor: usize = 0x054;
    pub const W_RXBufWriteAddr: usize = 0x056;
    pub const W_RXBufReadAddr: usize = 0x058;
    pub const W_RXBufReadCursor: usize = 0x05A;
    pub const W_RXBufCount: usize = 0x05C;
    pub const W_RXBufDataRead: usize = 0x060;
    pub const W_RXBufGapAddr: usize = 0x062;
    pub const W_RXBufGapSize: usize = 0x064;
    pub const W_TXBufWriteAddr: usize = 0x068;
    pub const W_TXBufCount: usize = 0x06C;
    pub const W_TXBufDataWrite: usize = 0x070;
    pub const W_TXBufGapAddr: usize = 0x074;
    pub const W_TXBufGapSize: usize = 0x076;
    pub const W_TXSlotBeacon: usize = 0x080;
    pub const W_TXBeaconTIM: usize = 0x084;
    pub const W_ListenCount: usize = 0x088;
    pub const W_BeaconInterval: usize = 0x08C;
    pub const W_ListenInterval: usize = 0x08E;
    pub const W_TXSlotCmd: usize = 0x090;
    pub const W_TXSlotReply1: usize = 0x094;
    pub const W_TXSlotReply2: usize = 0x098;
    pub const W_TXSlotLoc1: usize = 0x0A0;
    pub const W_TXSlotLoc2: usize = 0x0A4;
    pub const W_TXSlotLoc3: usize = 0x0A8;
    pub const W_TXReqReset: usize = 0x0AC;
    pub const W_TXReqSet: usize = 0x0AE;
    pub const W_TXReqRead: usize = 0x0B0;
    pub const W_TXSlotReset: usize = 0x0B4;
    pub const W_TXBusy: usize = 0x0B6;
    pub const W_TXStat: usize = 0x0B8;
    pub const W_Preamble: usize = 0x0BC;
    pub const W_CmdTotalTime: usize = 0x0C0;
    pub const W_CmdReplyTime: usize = 0x0C4;
    pub const W_RXFilter: usize = 0x0D0;
    pub const W_RXLenCrop: usize = 0x0DA;
    pub const W_RXFilter2: usize = 0x0E0;
    pub const W_USCountCnt: usize = 0x0E8;
    pub const W_USCompareCnt: usize = 0x0EA;
    pub const W_CmdCountCnt: usize = 0x0EE;
    pub const W_USCount0: usize = 0x0F8;
    pub const W_USCount1: usize = 0x0FA;
    pub const W_USCount2: usize = 0x0FC;
    pub const W_USCount3: usize = 0x0FE;
    pub const W_USCompare0: usize = 0x0F0;
    pub const W_USCompare1: usize = 0x0F2;
    pub const W_USCompare2: usize = 0x0F4;
    pub const W_USCompare3: usize = 0x0F6;
    pub const W_ContentFree: usize = 0x10C;
    pub const W_PreBeacon: usize = 0x110;
    pub const W_CmdCount: usize = 0x118;
    pub const W_BeaconCount1: usize = 0x11C;
    pub const W_BeaconCount2: usize = 0x134;
    pub const W_BBCnt: usize = 0x158;
    pub const W_BBWrite: usize = 0x15A;
    pub const W_BBRead: usize = 0x15C;
    pub const W_BBBusy: usize = 0x15E;
    pub const W_BBMode: usize = 0x160;
    pub const W_BBPower: usize = 0x168;
    pub const W_RFData2: usize = 0x17C;
    pub const W_RFData1: usize = 0x17E;
    pub const W_RFBusy: usize = 0x180;
    pub const W_RFCnt: usize = 0x184;
    pub const W_TXHeaderCnt: usize = 0x194;
    pub const W_RFPins: usize = 0x19C;
    pub const W_RXStatIncIF: usize = 0x1A8;
    pub const W_RXStatIncIE: usize = 0x1AA;
    pub const W_RXStatHalfIF: usize = 0x1AC;
    pub const W_RXStatHalfIE: usize = 0x1AE;
    pub const W_TXErrorCount: usize = 0x1C0;
    pub const W_RXCount: usize = 0x1C4;
    /// Base of the per-client MP reply-failure counters. These are **byte**-wide
    /// counters, two per 16-bit port, covering association IDs `1..=15` across
    /// ports `1D0h`..`1DEh` (`docs/design/melonds/Wifi.h:146-153`). melonDS
    /// addresses them as a flat byte array via `IOPORT8(W_CMDStat0 + i)`.
    pub const W_CMDStat0: usize = 0x1D0;
    pub const W_TXSeqNo: usize = 0x210;
    pub const W_RFStatus: usize = 0x214;
    pub const W_IFSet: usize = 0x21C;
    pub const W_RXTXAddr: usize = 0x268;
}
pub use names::*;

/// Registers on the path from "a client associated" to "an MP command round is
/// running". Traced as a group by the association trace; see [`write16`].
///
/// Deliberately excludes the RX ring and the BB/RF registers: those are busy
/// during normal operation and would bury the handful of writes that mark
/// actual progress.
///
/// [`write16`]: Wifi::write16
const MP_SETUP_REGS: [usize; 16] = [
    W_ModeWEP,
    W_BSSID0,
    W_AIDFull,
    W_TXSlotBeacon,
    W_TXBeaconTIM,
    W_ListenCount,
    W_BeaconInterval,
    // All three general-purpose slots, not just the first. The previous trace
    // covered only `W_TXSlotLoc1` and showed no writes at all, which reads as
    // "the driver never arms a slot" -- but the driver was arming LOC2/LOC3,
    // which the trace could not see. A partial view of a register group
    // invites exactly that conclusion.
    W_TXSlotLoc1,
    W_TXSlotLoc2,
    W_TXSlotLoc3,
    W_TXSlotReset,
    W_TXReqSet,
    W_TXReqReset,
    W_RXCnt,
    W_CmdTotalTime,
    W_CmdReplyTime,
];

/// Short label for a [`MP_SETUP_REGS`] entry, so the trace reads as register
/// names rather than offsets.
fn mp_setup_reg_name(reg: usize) -> &'static str {
    match reg {
        W_ModeWEP => "W_ModeWEP",
        W_BSSID0 => "W_BSSID0",
        W_AIDFull => "W_AIDFull",
        W_TXSlotBeacon => "W_TXSlotBeacon",
        W_TXBeaconTIM => "W_TXBeaconTIM",
        W_ListenCount => "W_ListenCount",
        W_BeaconInterval => "W_BeaconInterval",
        W_TXSlotLoc1 => "W_TXSlotLoc1",
        W_TXSlotLoc2 => "W_TXSlotLoc2",
        W_TXSlotLoc3 => "W_TXSlotLoc3",
        W_TXSlotReset => "W_TXSlotReset",
        W_RXCnt => "W_RXCnt",
        W_TXReqSet => "W_TXReqSet",
        W_TXReqReset => "W_TXReqReset",
        W_CmdTotalTime => "W_CmdTotalTime",
        W_CmdReplyTime => "W_CmdReplyTime",
        _ => "?",
    }
}

impl Wifi {
    /// 16-bit read from Wi-Fi address space (`addr` relative to
    /// `4800000h`). Implements the mirroring rules of
    /// `docs/design/design_lan.md` §6.2.
    ///
    /// Takes `request` because reading `W_RXBufDataRead` decrements
    /// `W_RXBufCount` and raises IRQ 9 on the zero transition -- a read with
    /// an interrupt side effect. See
    /// `docs/design/local-mp-melonds-parity-2.md` F4.
    pub fn read16(&mut self, addr: u32, request: &mut InterruptRequest) -> u16 {
        if addr >= 0x0001_0000 {
            return 0;
        }
        let addr = addr & 0x7FFE;

        if (0x4000..0x6000).contains(&addr) {
            let off = (addr & 0x1FFE) as usize;
            return self.ram[off] as u16 | (self.ram[off + 1] as u16) << 8;
        }
        if (0x2000..0x4000).contains(&addr) || addr >= 0x6000 {
            return 0xFFFF;
        }

        // 0000h-1FFFh: register file, with 1000h-1FFFh a "passive" mirror
        // that must not trigger auto-increment side effects.
        let active = addr < 0x1000;
        let reg = (addr & 0x0FFE) as usize;
        if let Some(n) = self.reg_read_counts.get_mut(reg >> 1) {
            *n = n.saturating_add(1);
        }
        let value = self.read_register(reg, active, request);
        if super::debug_enabled() {
            eprintln!("[wifi] reg read  0x{reg:03X} -> 0x{value:04X}");
        }
        value
    }

    fn read_register(&mut self, reg: usize, active: bool, request: &mut InterruptRequest) -> u16 {
        match reg {
            W_Random => {
                // Not a cryptographically accurate LFSR; matches melonDS's
                // "good enough for games" generator.
                self.random =
                    (self.random & 0x1) ^ (((self.random & 0x3FF) << 1) | (self.random >> 10));
                self.random
            }
            W_Preamble => self.ioport(W_Preamble) & 0x0003,
            W_USCount0 => (self.us_counter & 0xFFFF) as u16,
            W_USCount1 => ((self.us_counter >> 16) & 0xFFFF) as u16,
            W_USCount2 => ((self.us_counter >> 32) & 0xFFFF) as u16,
            W_USCount3 => (self.us_counter >> 48) as u16,
            W_USCompare0 => (self.us_compare & 0xFFFF) as u16,
            W_USCompare1 => ((self.us_compare >> 16) & 0xFFFF) as u16,
            W_USCompare2 => ((self.us_compare >> 32) & 0xFFFF) as u16,
            W_USCompare3 => (self.us_compare >> 48) as u16,
            W_CmdCount => self.cmd_counter.div_ceil(10) as u16,
            W_BBRead => {
                if (self.ioport(W_BBCnt) & 0xF000) != 0x6000 {
                    0
                } else {
                    self.bb_regs[(self.ioport(W_BBCnt) & 0xFF) as usize] as u16
                }
            }
            W_BBBusy | W_RFBusy => 0,
            // Hardware exposes no `W_TXBusy` bit for the automatic MP reply
            // slot: bit 7 is internal state only. `Wifi.cpp:2088` ("no bit for
            // MP replies. odd"). See
            // `docs/design/local-mp-melonds-parity-2.md` F2.
            W_TXBusy => self.ioport(W_TXBusy) & 0x001F,
            W_RXBufDataRead if active => self.rx_buf_data_read(request),
            _ => self.ioport(reg),
        }
    }

    /// Reads one halfword out of the RX ring at `W_RXBufReadAddr`, advancing
    /// the cursor past the ring end and over the driver-programmed gap.
    /// Ported from `Wifi.cpp:2057-2085`.
    ///
    /// The gap (`W_RXBufGapAddr`/`W_RXBufGapSize`) lets the driver read a
    /// frame's header and body as one contiguous stream while the hardware
    /// skips the region between them; ignoring it hands the driver the wrong
    /// bytes from the second field onward. `W_RXBufCount` counts down the
    /// halfwords the driver asked for and raises IRQ 9 on reaching zero -- the
    /// "receive buffer drained" signal a driver may block on. Neither was
    /// implemented here, even though the *write* path (`W_RXBufDataRead`'s
    /// write case below) already had both. See
    /// `docs/design/local-mp-melonds-parity-2.md` F4.
    fn rx_buf_data_read(&mut self, request: &mut InterruptRequest) -> u16 {
        let begin = self.ioport(W_RXBufBegin) as u32 & 0x1FFE;
        let end = self.ioport(W_RXBufEnd) as u32 & 0x1FFE;

        let mut rdaddr = self.ioport(W_RXBufReadAddr) as u32 & 0x1FFE;
        let ret = self.ram[rdaddr as usize] as u16 | (self.ram[rdaddr as usize + 1] as u16) << 8;

        // Armed by `Wifi::step_rx` when an association response is committed:
        // this is the driver reading it back, and the addresses say whether it
        // is looking where the frame actually landed. See
        // [`super::assoc_trace_enabled`].
        if self.assoc_trace_reads > 0 {
            self.assoc_trace_reads -= 1;
            eprintln!(
                "[assoc-trace][{:04X}]   driver read @0x{rdaddr:04X} -> 0x{ret:04X} \
                 (count={}, gap@0x{:04X} size={})",
                self.ioport(W_MACAddr2),
                self.ioport(W_RXBufCount),
                self.ioport(W_RXBufGapAddr),
                self.ioport(W_RXBufGapSize),
            );
        }

        rdaddr += 2;
        if rdaddr == end {
            rdaddr = begin;
        }
        if rdaddr == self.ioport(W_RXBufGapAddr) as u32 & 0x1FFE {
            rdaddr += (self.ioport(W_RXBufGapSize) as u32) << 1;
            if rdaddr >= end {
                rdaddr = rdaddr + begin - end;
            }
            // On the later Wi-Fi variant (DS Lite / DSi, `W_ID == 0xC340`) the
            // gap is consumed by crossing it once: the hardware clears
            // `W_RXBufGapSize` so the *next* pass over the same address reads
            // straight through. Ported from `Wifi.cpp:2072-2073`.
            //
            // Leaving it set makes every later read skip the gap again, so
            // from the second frame onward the driver is handed bytes from the
            // wrong offset while every length and filter check still passes.
            if self.is_modern_wifi() {
                self.set_ioport(W_RXBufGapSize, 0);
            }
        }
        self.set_ioport(W_RXBufReadAddr, (rdaddr & 0x1FFE) as u16);

        // Only decrement a non-zero count: melonDS never underflows this to
        // `0xFFFF`, and IRQ 9 fires exactly on the zero transition.
        let count = self.ioport(W_RXBufCount);
        if count > 0 {
            self.set_ioport(W_RXBufCount, count - 1);
            if count - 1 == 0 {
                self.raise_irq(9, request);
            }
        }

        ret
    }

    /// 16-bit write to Wi-Fi address space. Takes the scheduler because a
    /// `W_PowerUS`/`W_PowerForce` write can transition the hardware's
    /// power-on state, which must (re-)schedule or cancel
    /// [`super::scheduler::Event::Wifi`] -- see [`Wifi::update_power_on`].
    /// Takes `request` because `W_IE`/`W_IFSet` writes must re-evaluate the
    /// pending-interrupt edge synchronously (`Wifi::check_irq_edge`), and a
    /// `W_TXReqSet`/`W_RXCnt` write can immediately start a TX slot whose
    /// completion later raises an interrupt.
    pub fn write16(
        &mut self,
        addr: u32,
        value: u16,
        scheduler: &mut Scheduler,
        request: &mut InterruptRequest,
    ) {
        if addr >= 0x0001_0000 {
            return;
        }
        let addr = addr & 0x7FFE;

        if (0x4000..0x6000).contains(&addr) {
            let off = (addr & 0x1FFE) as usize;
            self.ram[off] = value as u8;
            self.ram[off + 1] = (value >> 8) as u8;
            return;
        }
        if (0x2000..0x4000).contains(&addr) || addr >= 0x6000 {
            return;
        }

        let reg = (addr & 0x0FFE) as usize;
        if super::debug_enabled() {
            eprintln!("[wifi] reg write 0x{reg:03X} <- 0x{value:04X}");
        }

        // Every register the driver touches on the way to an MP round, traced
        // as one stream. Reads are far too numerous to log and say little --
        // a driver polling a status register looks identical whether it is
        // making progress or spinning. *Writes* are rare and each one marks a
        // step its state machine actually completed, so the last write before
        // a teardown names how far it got. See
        // `docs/design/review_mp_local2.md` §7.1g.
        if super::assoc_trace_enabled() && MP_SETUP_REGS.contains(&reg) {
            eprintln!(
                "[assoc-trace][{:04X}] W[{}] = 0x{value:04X} (us_timestamp={})",
                self.ioport(W_MACAddr2),
                mp_setup_reg_name(reg),
                self.us_timestamp,
            );
        }

        self.write_register(reg, value, scheduler, request);
    }

    fn write_register(
        &mut self,
        reg: usize,
        value: u16,
        scheduler: &mut Scheduler,
        request: &mut InterruptRequest,
    ) {
        match reg {
            // `Wifi.cpp:2118-2183`. Bit 13 and bit 14 each restore a documented
            // group of registers to hardware defaults; the DS driver relies on
            // this instead of programming them one by one. Leaving it unported
            // meant `W_RXBufBegin`/`W_RXBufEnd` stayed zero, and
            // `Wifi::check_rx`'s zero-size-ring guard then rejected *every*
            // inbound frame. See `docs/design/local-mp-melonds-parity-2.md` F0.
            //
            // Not ported: the bit-0 edge's `UpdatePowerStatus(0)` call and the
            // `IOPORT(0x27C)` writes -- both belong to the power-management
            // state machine this emulator does not implement (see the previous
            // design document's Appendix C).
            W_ModeReset => {
                let old = self.ioport(W_ModeReset);
                // Bit 14 restores the register defaults, which includes zeroing
                // `W_AIDLow`. Logged so a trace shows the *order*: association
                // accepted, then torn down, is a completely different fault
                // from association never accepted.
                if super::assoc_trace_enabled() && value & 0x4000 != 0 {
                    eprintln!(
                        "[assoc-trace][{:04X}] driver re-initialised (W_ModeReset=0x{value:04X}), \
                         clearing W_AIDLow (was {}) (us_timestamp={})",
                        self.ioport(W_MACAddr2),
                        self.ioport(W_AIDLow),
                        self.us_timestamp,
                    );
                }
                self.set_ioport(W_ModeReset, value & 0x0001);
                // Bit 0 is the transceiver's master enable; both edges
                // re-evaluate power (`Wifi.cpp:2130-2143`).
                //
                // The rising edge requests power *on* rather than merely
                // re-evaluating. melonDS passes 0 on both edges, but it also
                // models `IOPORT(0x27C)` and a driver that drives
                // `W_PowerState` in mode 3. Here, clearing bit 0 forces the
                // radio off (`update_power_status`'s master-enable branch)
                // and re-asserting it would otherwise leave `reqflags` at 0
                // -- the radio never comes back, `check_rx` returns at its
                // `W_PowerState` bit-9 guard, and the instance stops
                // receiving for good. Asserting the master enable has to be
                // able to undo what clearing it did.
                // Both edges publish a status word at port `27Ch` and
                // re-evaluate power (`Wifi.cpp:2131-2143`). `27Ch` is not
                // one of the named `W_*` registers; melonDS writes it as a
                // bare port and so does this.
                //
                // The rising edge requests power *on* rather than merely
                // re-evaluating (melonDS passes 0 on both edges). Paired
                // with the deviation in `Wifi::update_power_status`, this is
                // what lets the master enable undo its own force-off:
                // without it `reqflags` stays 0 and the transmit half never
                // returns.
                if old & 0x0001 == 0 && value & 0x0001 != 0 {
                    self.set_ioport(0x27C, 0x0005);
                    self.update_power_status(1);
                } else if old & 0x0001 != 0 && value & 0x0001 == 0 {
                    self.set_ioport(0x27C, 0x000A);
                    self.update_power_status(0);
                }

                if value & 0x2000 != 0 {
                    self.set_ioport(W_RXBufWriteAddr, 0);
                    self.set_ioport(W_CmdTotalTime, 0);
                    self.set_ioport(W_CmdReplyTime, 0);
                    self.set_ioport(0x1A4, 0);
                    self.set_ioport(0x278, 0x000F);
                }
                if value & 0x4000 != 0 {
                    self.diag.mode_reset += 1;
                    self.set_ioport(W_ModeWEP, 0);
                    self.set_ioport(W_TXStatCnt, 0);
                    self.set_ioport(0x00A, 0);
                    self.set_ioport(W_MACAddr0, 0);
                    self.set_ioport(W_MACAddr1, 0);
                    self.set_ioport(W_MACAddr2, 0);
                    self.set_ioport(W_BSSID0, 0);
                    self.set_ioport(W_BSSID1, 0);
                    self.set_ioport(W_BSSID2, 0);
                    self.set_ioport(W_AIDLow, 0);
                    self.set_ioport(W_AIDFull, 0);
                    self.set_ioport(W_TXRetryLimit, 0x0707);
                    self.set_ioport(0x02E, 0);
                    self.set_ioport(W_RXBufBegin, 0x4000);
                    self.set_ioport(W_RXBufEnd, 0x4800);
                    self.set_ioport(W_TXBeaconTIM, 0);
                    self.set_ioport(W_Preamble, 0x0001);
                    self.set_ioport(W_RXFilter, 0x0401);
                    self.set_ioport(0x0D4, 0x0001);
                    self.set_ioport(W_RXFilter2, 0x0008);
                    self.set_ioport(0x0EC, 0x3F03);
                    self.set_ioport(W_TXHeaderCnt, 0);
                    self.set_ioport(0x198, 0);
                    self.set_ioport(0x1A2, 0x0001);
                    self.set_ioport(0x224, 0x0003);
                    self.set_ioport(0x230, 0x0047);
                }
            }
            W_IF => self.set_ioport(W_IF, self.ioport(W_IF) & !value),
            // Enabling an already-pending flag must re-raise the ARM7
            // request on this write, not wait for the next `SetIRQ` call.
            // `Wifi.cpp:2204-2210`.
            W_IE => {
                let old_flags = self.ioport(W_IF) & self.ioport(W_IE);
                self.set_ioport(W_IE, value);
                self.check_irq_edge(old_flags, request);
            }
            W_IFSet => {
                let old_flags = self.ioport(W_IF) & self.ioport(W_IE);
                self.set_ioport(W_IF, self.ioport(W_IF) | (value & 0xFBFF));
                self.check_irq_edge(old_flags, request);
            }
            // The single fact that splits "the driver rejected the association"
            // from "the driver accepted it and the link died later": after a
            // re-initialisation, `W_ModeReset` bit 14 zeroes `W_AIDLow`, so a
            // diagnostic snapshot reading `aid 0` proves nothing on its own.
            // Only the write itself does. See [`super::assoc_trace_enabled`].
            W_AIDLow => {
                if super::assoc_trace_enabled() {
                    eprintln!(
                        "[assoc-trace][{:04X}] driver wrote W_AIDLow = {} (us_timestamp={})",
                        self.ioport(W_MACAddr2),
                        value & 0x000F,
                        self.us_timestamp,
                    );
                }
                self.set_ioport(W_AIDLow, value & 0x000F);
            }
            W_AIDFull => self.set_ioport(W_AIDFull, value & 0x07FF),
            W_TXBufDataWrite => self.tx_buf_data_write(value, request),
            // `W_BBWrite` only stages the value; the transfer is committed by
            // the following `W_BBCnt` write. See [`Wifi::bb_write`].
            W_BBWrite => self.set_ioport(W_BBWrite, value),
            W_BBCnt => self.bb_write(value),
            W_RFData1 | W_RFData2 => {
                self.set_ioport(reg, value);
                if reg == W_RFData2 {
                    self.rf_transfer();
                }
            }
            // `W_TXReqRead` (not `W_TXBusy`, which is read-only to the CPU)
            // records which slots the driver has asked to start; `fire_tx`
            // decides whether any of them can actually begin now. See
            // `docs/design/local-mp-melonds-parity.md` Gap 1.2.
            W_TXReqSet => {
                self.set_ioport(W_TXReqRead, self.ioport(W_TXReqRead) | value);
                self.fire_tx();
            }
            W_TXReqReset => {
                self.set_ioport(W_TXReqRead, self.ioport(W_TXReqRead) & !value);
            }
            W_TXSlotReset => {
                if value & 0x0001 != 0 {
                    self.set_ioport(W_TXSlotLoc1, self.ioport(W_TXSlotLoc1) & 0x7FFF);
                }
                if value & 0x0002 != 0 {
                    self.set_ioport(W_TXSlotCmd, self.ioport(W_TXSlotCmd) & 0x7FFF);
                }
                if value & 0x0004 != 0 {
                    self.set_ioport(W_TXSlotLoc2, self.ioport(W_TXSlotLoc2) & 0x7FFF);
                }
                if value & 0x0008 != 0 {
                    self.set_ioport(W_TXSlotLoc3, self.ioport(W_TXSlotLoc3) & 0x7FFF);
                }
                if value & 0x0040 != 0 {
                    self.set_ioport(W_TXSlotReply2, self.ioport(W_TXSlotReply2) & 0x7FFF);
                }
                if value & 0x0080 != 0 {
                    self.set_ioport(W_TXSlotReply1, self.ioport(W_TXSlotReply1) & 0x7FFF);
                }
                // Write-only port; melonDS stores 0 back regardless of `value`.
            }
            // Slot address registers: latching the value (with `W_TXSlotCmd`'s
            // "keep bit 15 if `CmdCounter` is still zero" quirk) then calling
            // `fire_tx` is what actually starts a transmission -- a plain
            // register write with no side effect (the previous behaviour)
            // never transmits anything. `Wifi.cpp:2425-2436`.
            W_TXSlotCmd => {
                self.diag.tx_slot_cmd_writes += 1;
                // The host arming an MP command round is the first thing that
                // should happen once a client has associated. If association
                // succeeds but this never fires, the link is dying between the
                // two, not at either end.
                if super::assoc_trace_enabled() {
                    eprintln!(
                        "[assoc-trace][{:04X}] driver wrote W_TXSlotCmd = 0x{value:04X} \
                         (cmd_counter={}, us_timestamp={})",
                        self.ioport(W_MACAddr2),
                        self.cmd_counter,
                        self.us_timestamp,
                    );
                }
                let value = if self.cmd_counter == 0 {
                    if value & 0x8000 != 0 && self.ioport(W_TXSlotCmd) & 0x8000 == 0 {
                        self.diag.tx_slot_cmd_bit15_dropped += 1;
                    }
                    (value & 0x7FFF) | (self.ioport(W_TXSlotCmd) & 0x8000)
                } else {
                    value
                };
                self.set_ioport(W_TXSlotCmd, value);
                self.fire_tx();
            }
            W_TXSlotLoc1 | W_TXSlotLoc2 | W_TXSlotLoc3 => {
                self.set_ioport(reg, value);
                self.fire_tx();
            }
            // `Wifi.cpp:2421-2423`: this is the only place `is_mp` is set on
            // the host side. Without it the host never believes it is
            // engaged in an MP session at all. See Gap 1.3.
            W_TXSlotBeacon => {
                self.is_mp = (value & 0x8000) != 0;
                self.set_ioport(W_TXSlotBeacon, value);
            }
            // `Wifi.cpp:2329-2345`. Bit 0 resets the write cursor to the
            // current write address; bit 7 hands the staged reply slot off
            // to the "in flight" reply-2 register; bit 15 kicks `fire_tx`.
            W_RXCnt => {
                if value & 0x0001 != 0 {
                    self.set_ioport(W_RXBufWriteCursor, self.ioport(W_RXBufWriteAddr));
                }
                if value & 0x0080 != 0 {
                    self.set_ioport(W_TXSlotReply2, self.ioport(W_TXSlotReply1));
                    self.set_ioport(W_TXSlotReply1, 0);
                }
                self.set_ioport(W_RXCnt, value & 0xFF0E);
                if value & 0x8000 != 0 {
                    self.fire_tx();
                }
            }
            W_RXBufDataRead => {
                let count = self.ioport(W_RXBufCount);
                if count > 0 {
                    let count = count - 1;
                    self.set_ioport(W_RXBufCount, count);
                    if count == 0 {
                        self.raise_irq(9, request);
                    }
                }
            }
            W_RXBufReadAddr | W_TXBufWriteAddr | W_RXBufGapAddr => {
                self.set_ioport(reg, value & 0x1FFE);
            }
            // The driver has now told the hardware where it has read up to, so
            // `Wifi::start_rx`'s ring-overrun check has a meaningful cursor to
            // compare against. Before this first write the register reads zero,
            // which after the halfword shift is also the ring base -- see
            // [`Wifi::rx_read_cursor_written`] and
            // `docs/design/review_mp_local2.md` P0-4.
            W_RXBufReadCursor => {
                self.rx_read_cursor_written = true;
                self.set_ioport(reg, value & 0x0FFF);
            }
            W_RXBufGapSize | W_RXBufCount | W_RXBufWriteAddr | W_TXBufGapSize | W_TXBufCount => {
                self.set_ioport(reg, value & 0x0FFF);
            }
            // Counted for the `diag` summary: if neither this nor a
            // `W_ModeReset` bit-14 write ever happens, the RX ring stays
            // zero-sized and `check_rx` rejects every inbound frame.
            W_RXBufBegin | W_RXBufEnd => {
                self.diag.rxbuf_cfg += 1;
                self.set_ioport(reg, value);
            }
            // Writes to `W_CmdCount` set the countdown timer directly
            // (reads recompute the driver-visible value from it via
            // `div_ceil`); the register mirror itself is never stored.
            // `Wifi.cpp:2307`.
            // The host programming the CMD-round budget. Traced alongside
            // `W_TXSlotCmd` because these two are the last steps before MP
            // rounds begin: seeing this but not `W_TXSlotCmd` means the driver
            // got part-way and stopped, which is a different fault from never
            // starting at all.
            W_CmdCount => {
                if super::assoc_trace_enabled() {
                    eprintln!(
                        "[assoc-trace][{:04X}] driver wrote W_CmdCount = {value} \
                         (us_timestamp={})",
                        self.ioport(W_MACAddr2),
                        self.us_timestamp,
                    );
                }
                self.diag.cmd_count_writes += 1;
                self.cmd_counter = u32::from(value) * 10;
            }
            // `Wifi.cpp:2230-2233`.
            W_PowerUS => {
                self.set_ioport(W_PowerUS, value & 0x0003);
                self.update_power_on(scheduler);
            }
            W_PowerForce => {
                self.set_ioport(W_PowerForce, value & 0x8001);
                self.update_power_on(scheduler);
                self.update_power_status(0);
            }
            // `Wifi.cpp:2186-2202`: selecting a power-management mode can
            // arm `W_PowerDownCtrl` and clear the transceiver state.
            W_ModeWEP => {
                let value = value & 0x007F;
                self.set_ioport(W_ModeWEP, value);
                if self.ioport(W_PowerTX) & (1 << 1) != 0 {
                    match value & 0x7 {
                        1 => self
                            .set_ioport(W_PowerDownCtrl, self.ioport(W_PowerDownCtrl) | (1 << 1)),
                        2 => self.set_ioport(W_PowerDownCtrl, 3),
                        _ => {}
                    }
                    if value & 0x7 != 3 {
                        self.set_ioport(W_PowerState, self.ioport(W_PowerState) & 0x0300);
                    }
                    self.update_power_status(0);
                }
            }
            // `Wifi.cpp:2236-2246`.
            W_PowerTX => {
                self.set_ioport(W_PowerTX, value & 0x0003);
                if value & (1 << 1) != 0 {
                    match self.ioport(W_ModeWEP) & 0x7 {
                        1 => self
                            .set_ioport(W_PowerDownCtrl, self.ioport(W_PowerDownCtrl) | (1 << 1)),
                        2 => self.set_ioport(W_PowerDownCtrl, 3),
                        _ => {}
                    }
                    self.update_power_status(0);
                }
            }
            // The driver's main "power the radio up/down" port; only
            // writable in `W_ModeWEP` mode 3 (`Wifi.cpp:2249-2264`).
            W_PowerState => {
                if self.ioport(W_ModeWEP) & 0x7 != 3 {
                    return;
                }
                let mut v = (self.ioport(W_PowerState) & 0x0300) | (value & 0x0003);
                if v & 0x0300 == 0x0200 {
                    v &= !(1 << 0);
                } else {
                    v &= !(1 << 1);
                }
                if v & (1 << 9) == 0 {
                    v &= !(1 << 8);
                }
                self.set_ioport(W_PowerState, v);
                self.update_power_status(0);
            }
            // `Wifi.cpp:2271-2286`.
            W_PowerDownCtrl => {
                self.set_ioport(W_PowerDownCtrl, value & 0x0003);
                if self.ioport(W_PowerTX) & (1 << 1) != 0 {
                    match self.ioport(W_ModeWEP) & 0x7 {
                        1 => self
                            .set_ioport(W_PowerDownCtrl, self.ioport(W_PowerDownCtrl) | (1 << 1)),
                        2 => self.set_ioport(W_PowerDownCtrl, 3),
                        _ => {}
                    }
                }
                self.update_power_status(0);
            }
            // `W_USCount0..3` back the `us_counter` field directly, mirroring
            // melonDS's `Wifi.cpp:2294-2297`; without this the CPU cannot
            // ever set the MP sync clock's millisecond half.
            W_USCount0 => self.us_counter = (self.us_counter & !0xFFFF) | u64::from(value),
            W_USCount1 => {
                self.us_counter = (self.us_counter & !0xFFFF_0000) | (u64::from(value) << 16);
            }
            W_USCount2 => {
                self.us_counter = (self.us_counter & !0xFFFF_0000_0000) | (u64::from(value) << 32);
            }
            W_USCount3 => {
                self.us_counter = (self.us_counter & 0xFFFF_FFFF_FFFF) | (u64::from(value) << 48);
            }
            // `W_USCompare0..3` back the `us_compare` field the beacon
            // timer's `W_USCompareCnt` IRQ compares against (see
            // `Wifi::ms_timer`); the generic fallback below only updates
            // the raw register mirror, not that field, so writes here
            // would otherwise never take effect.
            W_USCompare0 => {
                self.set_ioport(reg, value & 0xFC00);
                self.us_compare = (self.us_compare & !0xFFFF) | u64::from(value & 0xFC00);
                // Bit 0 is not part of the compare value: it suppresses the
                // beacon-interval source of IRQ 14 until `USCOUNTER` reaches
                // the timestamp being programmed here. The driver uses it to
                // replace the free-running beacon interrupt with a one-shot
                // wake-up, which is how a host schedules its beacon and MP
                // timing. Ported from `Wifi.cpp:2300-2302`; leaving it
                // unimplemented meant the beacon interrupt kept firing through
                // a window the driver had explicitly asked to be quiet.
                if value & 0x0001 != 0 {
                    self.block_beacon_irq14 = true;
                }
            }
            W_USCompare1 => {
                self.set_ioport(reg, value);
                self.us_compare = (self.us_compare & !0xFFFF_0000) | (u64::from(value) << 16);
            }
            W_USCompare2 => {
                self.set_ioport(reg, value);
                self.us_compare = (self.us_compare & !0xFFFF_0000_0000) | (u64::from(value) << 32);
            }
            W_USCompare3 => {
                self.set_ioport(reg, value);
                self.us_compare = (self.us_compare & 0xFFFF_FFFF_FFFF) | (u64::from(value) << 48);
            }
            // Read-only ports: the CPU cannot write these. `Wifi.cpp:2443-2462`.
            W_ID | W_TRXPower | W_Random | W_RXBufWriteCursor | W_TXSlotReply2 | W_TXReqRead
            | W_TXBusy | W_TXStat | W_BBRead | W_BBBusy | W_RFBusy | W_RFPins | W_RXStatIncIF
            | W_RXStatHalfIF | W_RXCount | W_TXSeqNo | W_RFStatus | W_RXTXAddr => {}
            _ => self.set_ioport(reg, value),
        }
    }

    /// Writes one halfword of a staged TX frame at `W_TXBufWriteAddr`,
    /// advancing the cursor over the driver-programmed gap. Ported from
    /// `Wifi.cpp:2389-2400`.
    ///
    /// The driver stages a frame's 12-byte hardware header and its 802.11 body
    /// as one contiguous write stream; `W_TXBufGapAddr`/`W_TXBufGapSize` tell
    /// the hardware to skip the region between them. Ignoring the gap lays
    /// every staged frame out wrong in Wi-Fi RAM. See
    /// `docs/design/local-mp-melonds-parity-2.md` F4.
    fn tx_buf_data_write(&mut self, value: u16, request: &mut InterruptRequest) {
        let mut addr = self.ioport(W_TXBufWriteAddr) as u32 & 0x1FFE;
        self.ram[addr as usize] = value as u8;
        self.ram[addr as usize + 1] = (value >> 8) as u8;
        addr += 2;
        if addr == self.ioport(W_TXBufGapAddr) as u32 & 0x1FFE {
            addr += (self.ioport(W_TXBufGapSize) as u32) << 1;
        }
        self.set_ioport(W_TXBufWriteAddr, addr as u16 & 0x1FFE);

        let count = self.ioport(W_TXBufCount);
        if count > 0 {
            let count = count - 1;
            self.set_ioport(W_TXBufCount, count);
            if count == 0 {
                self.raise_irq(8, request);
            }
        }
    }

    /// Commits a baseband register transfer. Ported from melonDS's
    /// `case W_BBCnt` (`Wifi.cpp:2309-2317`).
    ///
    /// The driver stages the byte in `W_BBWrite` and *then* writes `W_BBCnt`
    /// with `5000h | regid` to perform the transfer, so the commit belongs on
    /// the `W_BBCnt` write. This module used to commit on the `W_BBWrite`
    /// write instead, using whatever `W_BBCnt` still held from the
    /// **previous** transfer -- so every value landed in the wrong register.
    /// The driver reads each register back through `W_BBRead` to verify its
    /// upload, so it never saw what it wrote, and looped re-uploading the
    /// whole baseband table forever without ever reaching RF channel
    /// selection.
    fn bb_write(&mut self, value: u16) {
        self.set_ioport(W_BBCnt, value);
        if (value & 0xF000) == 0x5000 {
            self.diag.bb_writes += 1;
            let id = (value & 0xFF) as usize;
            // Some registers are hardwired and silently ignore writes
            // (`BBREG_FIXED`, `Wifi.cpp:119-146`).
            if self.bb_regs_ro[id] == 0 {
                self.bb_regs[id] = self.ioport(W_BBWrite) as u8;
            }
        }
    }

    /// 8-bit read: extracts the requested byte from the 16-bit read.
    pub fn read8(&mut self, addr: u32, request: &mut InterruptRequest) -> u8 {
        let word = self.read16(addr & !1, request);
        if addr & 1 != 0 { (word >> 8) as u8 } else { word as u8 }
    }

    /// 8-bit write: logged and dropped. No real game performs 8-bit writes
    /// to `W_*` registers; composing two 8-bit writes into one 16-bit access
    /// would double-fire side effects (`docs/design/design_lan.md` §6.2).
    pub fn write8(&mut self, addr: u32, value: u8) {
        if super::debug_enabled() {
            eprintln!("[wifi] IGNORED 8-bit write addr=0x{addr:08X} value=0x{value:02X}");
        }
        warn!("Ignoring 8-bit Wi-Fi register write at 0x{addr:08X} = 0x{value:02X}");
    }

    /// Raises `W_IF` bit `irq`, propagating to the ARM7 interrupt controller
    /// only on the `0 -> nonzero` edge of `W_IF & W_IE`.
    pub(super) fn raise_irq(&mut self, irq: u32, request: &mut InterruptRequest) {
        self.set_irq(irq, request);
    }
}
