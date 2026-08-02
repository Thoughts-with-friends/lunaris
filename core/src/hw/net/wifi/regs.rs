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
    pub const W_TXSeqNo: usize = 0x210;
    pub const W_RFStatus: usize = 0x214;
    pub const W_IFSet: usize = 0x21C;
    pub const W_RXTXAddr: usize = 0x268;
}
pub use names::*;

impl Wifi {
    /// 16-bit read from Wi-Fi address space (`addr` relative to
    /// `4800000h`). Implements the mirroring rules of
    /// `docs/design/design_lan.md` §6.2.
    pub fn read16(&mut self, addr: u32) -> u16 {
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
        let value = self.read_register(reg, active);
        if super::debug_enabled() {
            eprintln!("[wifi] reg read  0x{reg:03X} -> 0x{value:04X}");
        }
        value
    }

    fn read_register(&mut self, reg: usize, active: bool) -> u16 {
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
            W_RXBufDataRead if active => self.rx_buf_data_read(),
            _ => self.ioport(reg),
        }
    }

    fn rx_buf_data_read(&mut self) -> u16 {
        let mut rdaddr = self.ioport(W_RXBufReadAddr) as u32 & 0x1FFE;
        let ret = self.ram[rdaddr as usize] as u16 | (self.ram[rdaddr as usize + 1] as u16) << 8;
        rdaddr += 2;
        if rdaddr == (self.ioport(W_RXBufEnd) as u32 & 0x1FFE) {
            rdaddr = self.ioport(W_RXBufBegin) as u32 & 0x1FFE;
        }
        self.set_ioport(W_RXBufReadAddr, rdaddr as u16);
        let count = self.ioport(W_RXBufCount).wrapping_sub(1);
        self.set_ioport(W_RXBufCount, count);
        ret
    }

    /// 16-bit write to Wi-Fi address space. Takes the scheduler because a
    /// `W_PowerUS`/`W_PowerForce` write can transition the hardware's
    /// power-on state, which must (re-)schedule or cancel
    /// [`super::scheduler::Event::Wifi`] -- see [`Wifi::update_power_on`].
    pub fn write16(&mut self, addr: u32, value: u16, scheduler: &mut Scheduler) {
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
        self.write_register(reg, value, scheduler);
    }

    fn write_register(&mut self, reg: usize, value: u16, scheduler: &mut Scheduler) {
        // if super::debug_enabled() {
        //     println!("[wifi write] reg={:#05X} value={:#06X}", reg, value);
        // }

        match reg {
            W_IF => self.set_ioport(W_IF, self.ioport(W_IF) & !value),
            W_TXBufDataWrite => self.tx_buf_data_write(value),
            W_BBWrite => self.bb_write(value),
            W_BBCnt => {
                self.set_ioport(W_BBCnt, value);
            }
            W_RFData1 | W_RFData2 => {
                self.set_ioport(reg, value);
                if reg == W_RFData2 {
                    self.rf_transfer();
                }
            }
            W_TXReqSet => {
                let busy = self.ioport(W_TXBusy) | value;
                self.set_ioport(W_TXBusy, busy);

                println!("try_start_tx value={:04X}", value);
                self.try_start_tx(value);
            }
            W_TXReqReset => {
                self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) & !value);
            }
            // `Wifi::tick` (and everything downstream of it -- beacons,
            // channel resolution, RX polling) only runs while `power_on`
            // is true, which is derived from *both* `POWCNT2` (handled in
            // `set_power_cnt`) and this register's bit 0 (power-save).
            // Real drivers commonly enable `POWCNT2` early during general
            // system init and only clear `W_PowerUS` bit 0 later, once
            // they're actually about to use the radio (e.g. entering a
            // Union Room) -- missing the re-check here left the hardware
            // silently stuck "off" for the rest of the session even though
            // the driver believed it had powered up. See
            // `docs/design/design_lan.md` §6.5 and the Union-Room-never-
            // sees-a-peer symptom this fixes.
            W_PowerUS => {
                self.set_ioport(W_PowerUS, value);
                self.update_power_on(scheduler);
            }
            W_PowerForce => {
                self.set_ioport(W_PowerForce, value);
                self.update_power_on(scheduler);
            }
            W_RXBufReadAddr | W_TXBufWriteAddr => {
                self.set_ioport(reg, value & 0x1FFE);
            }
            // `W_USCompare0..3` back the `us_compare` field the beacon
            // timer's `W_USCompareCnt` IRQ compares against (see
            // `Wifi::ms_timer`); the generic fallback below only updates
            // the raw register mirror, not that field, so writes here
            // would otherwise never take effect.
            W_USCompare0 => {
                self.set_ioport(reg, value);
                self.us_compare = (self.us_compare & !0xFFFF) | value as u64;
            }
            W_USCompare1 => {
                self.set_ioport(reg, value);
                self.us_compare = (self.us_compare & !0xFFFF_0000) | (value as u64) << 16;
            }
            W_USCompare2 => {
                self.set_ioport(reg, value);
                self.us_compare = (self.us_compare & !0xFFFF_0000_0000) | (value as u64) << 32;
            }
            W_USCompare3 => {
                self.set_ioport(reg, value);
                self.us_compare = (self.us_compare & 0xFFFF_FFFF_FFFF) | (value as u64) << 48;
            }
            _ => self.set_ioport(reg, value),
        }
    }

    fn tx_buf_data_write(&mut self, value: u16) {
        let mut addr = self.ioport(W_TXBufWriteAddr) as u32 & 0x1FFE;
        self.ram[addr as usize] = value as u8;
        self.ram[addr as usize + 1] = (value >> 8) as u8;
        addr += 2;
        self.set_ioport(W_TXBufWriteAddr, addr as u16 & 0x1FFE);
    }

    fn bb_write(&mut self, value: u16) {
        let cnt = self.ioport(W_BBCnt);
        if (cnt & 0xF000) == 0x5000 {
            let id = (cnt & 0xFF) as usize;
            if self.bb_regs_ro[id] == 0 {
                self.bb_regs[id] = value as u8;
            }
        }
    }

    /// 8-bit read: extracts the requested byte from the 16-bit read.
    pub fn read8(&mut self, addr: u32) -> u8 {
        let word = self.read16(addr & !1);
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
