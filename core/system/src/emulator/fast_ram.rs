//! Fast path for CPU accesses that land in plain RAM.
//!
//! Most loads, stores and every instruction fetch hit RAM, yet the full bus
//! path ([`Emulator::read_word`] and friends) first syncs devices, checks
//! Slot-2, the 2D/3D register windows and then walks the address map. For
//! memory without side effects none of that matters, so it is resolved here
//! in a few compares.
//!
//! # Scope (what this module may touch)
//!
//! ```text
//!  ARM9  ITCM / DTCM (CP15)          checked first, as on the bus
//!        main RAM  0x02xxxxxx
//!  ARM7  main RAM  0x02xxxxxx
//!        WRAM      0x03xxxxxx        shared (per WRAMCNT) or ARM7 WRAM
//!  anything else (BIOS, ARM9 shared WRAM, I/O, palette, VRAM, OAM,
//!  Slot-2, ...)  -> None / false: the caller takes the full bus path
//! ```
//!
//! # Contract
//!
//! * The result is byte-for-byte what the full path returns or stores
//!   (same masks and offsets as `read_arm7/9.rs` / `write_arm7/9.rs`).
//! * Nothing here has side effects beyond the RAM itself, so callers may
//!   use it from any context (CPU, HLE BIOS, a future DMA fast path).
//! * Writes step aside while the debug watchpoint
//!   (`Config::watch_addr`) is armed, so it still sees every store.
//! * An access running past the end of a RAM array is not handled here
//!   (`None` / `false`); the full path keeps its own behaviour for it.
use ds_cpu::arm_cpu::CpuType;
use ds_mem_addr::{IO_REGS_START, MAIN_RAM_MASK, MAIN_RAM_START, SHARED_WRAM_START};

use crate::emulator::Emulator;

/// Which RAM array a plain-RAM access lands in, and the offset there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RamSlot {
    /// 4 MiB main RAM (both CPUs).
    Main(usize),
    /// ARM7 private 64 KiB WRAM.
    Arm7Wram(usize),
    /// 32 KiB shared WRAM as banked for the ARM7 by WRAMCNT.
    SharedWram(usize),
}

impl Emulator {
    /// Resolves `address` to a plain-RAM slot for `cpu`, or `None` when the
    /// access needs the full bus path. TCM is not resolved here (see
    /// [`Self::fast_read`]).
    #[inline(always)]
    pub(crate) const fn plain_ram_slot(&self, address: u32, cpu: CpuType) -> Option<RamSlot> {
        if address >= MAIN_RAM_START && address < SHARED_WRAM_START {
            // DS Memory Maps: https://problemkaputt.de/gbatek.htm#dsmemorymaps
            return Some(RamSlot::Main((address & MAIN_RAM_MASK) as usize));
        }
        if matches!(cpu, CpuType::Arm7) && address >= SHARED_WRAM_START && address < IO_REGS_START
        {
            let (private, off) = self.arm7_wram_offset(address);
            return Some(if private { RamSlot::Arm7Wram(off) } else { RamSlot::SharedWram(off) });
        }
        None
    }

    /// The bytes behind `slot` (read side).
    #[inline(always)]
    fn ram_bytes(&self, slot: RamSlot) -> (&[u8], usize) {
        match slot {
            RamSlot::Main(off) => (&self.main_ram, off),
            RamSlot::Arm7Wram(off) => (&self.arm7_wram, off),
            RamSlot::SharedWram(off) => (&self.shared_wram, off),
        }
    }

    /// The bytes behind `slot` (write side).
    #[inline(always)]
    fn ram_bytes_mut(&mut self, slot: RamSlot) -> (&mut [u8], usize) {
        match slot {
            RamSlot::Main(off) => (&mut self.main_ram, off),
            RamSlot::Arm7Wram(off) => (&mut self.arm7_wram, off),
            RamSlot::SharedWram(off) => (&mut self.shared_wram, off),
        }
    }

    /// Reads `N` bytes (little endian) from TCM or plain RAM; `None` when
    /// `address` is outside this module's scope.
    #[inline(always)]
    pub(crate) fn fast_read<const N: usize>(&self, address: u32, cpu: CpuType) -> Option<[u8; N]> {
        if matches!(cpu, CpuType::Arm9)
            && let Some(bytes) = self.arm9_cp15.tcm_read::<N>(address)
        {
            return Some(bytes);
        }
        let (mem, off) = self.ram_bytes(self.plain_ram_slot(address, cpu)?);
        mem.get(off..off + N).map(|src| core::array::from_fn(|i| src[i]))
    }

    /// Stores `bytes` to TCM or plain RAM; `false` when `address` is
    /// outside this module's scope (nothing was written then).
    #[inline(always)]
    pub(crate) fn fast_write(&mut self, address: u32, bytes: &[u8], cpu: CpuType) -> bool {
        if self.config.watch_addr.is_some() {
            return false; // the full path reports watched stores
        }
        if matches!(cpu, CpuType::Arm9) && self.arm9_cp15.tcm_write(address, bytes) {
            return true;
        }
        let Some(slot) = self.plain_ram_slot(address, cpu) else { return false };
        let (mem, off) = self.ram_bytes_mut(slot);
        match mem.get_mut(off..off + bytes.len()) {
            Some(dst) => {
                dst.copy_from_slice(bytes);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fast path must agree with the full bus path everywhere it
    /// claims an address, for both CPUs and every WRAMCNT setting.
    #[test]
    fn fast_path_matches_full_path() {
        let mut emu = Emulator::new();
        for (i, b) in emu.main_ram.iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        for (i, b) in emu.arm7_wram.iter_mut().enumerate() {
            *b = (i * 11 + 5) as u8;
        }
        for (i, b) in emu.shared_wram.iter_mut().enumerate() {
            *b = (i * 13 + 1) as u8;
        }
        let addrs = [
            0x0200_0000,
            0x0212_3456,
            0x023F_FFF0,
            0x027F_FFF8, // mirror of main RAM
            0x0300_0010,
            0x0300_7FF0,
            0x037F_8000,
            0x0380_0000,
            0x0380_FFF0,
        ];
        for wram_cnt in 0..4 {
            emu.wram_cnt = wram_cnt;
            for cpu in [CpuType::Arm9, CpuType::Arm7] {
                for &a in &addrs {
                    if let Some(b) = emu.fast_read::<4>(a, cpu) {
                        let full = match cpu {
                            CpuType::Arm9 => emu.arm9_read_word(a),
                            CpuType::Arm7 => emu.arm7_read_word(a),
                        };
                        assert_eq!(u32::from_le_bytes(b), full, "{cpu:?} {a:08X} cnt {wram_cnt}");
                    }
                    if let Some(b) = emu.fast_read::<2>(a, cpu) {
                        let full = match cpu {
                            CpuType::Arm9 => emu.arm9_read_halfword(a),
                            CpuType::Arm7 => emu.arm7_read_halfword(a),
                        };
                        assert_eq!(u16::from_le_bytes(b), full, "{cpu:?} {a:08X} cnt {wram_cnt}");
                    }
                }
            }
        }
        // Out of scope: I/O and VRAM always take the full path.
        assert_eq!(emu.fast_read::<4>(0x0400_0000, CpuType::Arm7), None);
        assert_eq!(emu.fast_read::<4>(0x0600_0000, CpuType::Arm9), None);
        // ARM9 shared WRAM is left to the full path.
        assert_eq!(emu.plain_ram_slot(0x0300_0000, CpuType::Arm9), None);
    }

    #[test]
    fn fast_write_lands_where_the_full_path_reads() {
        let mut emu = Emulator::new();
        for cpu in [CpuType::Arm9, CpuType::Arm7] {
            assert!(emu.fast_write(0x0210_0000, &0xDEAD_BEEF_u32.to_le_bytes(), cpu));
            let full = match cpu {
                CpuType::Arm9 => emu.arm9_read_word(0x0210_0000),
                CpuType::Arm7 => emu.arm7_read_word(0x0210_0000),
            };
            assert_eq!(full, 0xDEAD_BEEF);
        }
        assert!(emu.fast_write(0x0380_0100, &[0x5A], CpuType::Arm7));
        assert_eq!(emu.arm7_read_byte(0x0380_0100), 0x5A);
        assert!(!emu.fast_write(0x0400_0208, &[1], CpuType::Arm9)); // IME: full path

        emu.config.watch_addr = Some(0x0210_0000);
        assert!(!emu.fast_write(0x0210_0000, &[0], CpuType::Arm9), "watch sees every store");
    }
}
