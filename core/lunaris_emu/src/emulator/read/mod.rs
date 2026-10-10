//! cpu.cpp:157
mod read_arm7;
mod read_arm9;

use lunaris_ds_mem_const::{ARM7_WRAM_MASK, ARM7_WRAM_START};

use crate::cpu::arm_cpu::CpuType;
use crate::emulator::Emulator;

impl Emulator {
    /// Resolves an ARM7 access in `0x03000000..0x04000000` to
    /// `(is_arm7_wram, offset)`.
    ///
    /// `0x03800000..` is always the ARM7's private 64 KiB WRAM. Below that,
    /// the 32 KiB shared WRAM is banked by WRAMCNT (GBATEK "DS Memory Control
    /// - WRAM"; melonDS `NDS::MapSharedWRAM`): 0 = none for ARM7 (the window
    ///
    /// mirrors ARM7 WRAM), 1 = first 16 KiB, 2 = second 16 KiB, 3 = all 32 KiB.
    pub(crate) const fn arm7_wram_offset(&self, address: u32) -> (bool, usize) {
        if address >= ARM7_WRAM_START {
            return (true, (address & ARM7_WRAM_MASK) as usize);
        }
        match self.wram_cnt & 3 {
            0 => (true, (address & ARM7_WRAM_MASK) as usize),
            1 => (false, (address & 0x3FFF) as usize),
            2 => (false, ((address & 0x3FFF) + 0x4000) as usize),
            _ => (false, (address & 0x7FFF) as usize),
        }
    }

    pub fn read_word(&mut self, address: u32, cpu_type: CpuType) -> u32 {
        if cpu_type == CpuType::Arm9 {
            match self.arm9_cp15.tcm_read::<4>(address) {
                Some(bytes) => u32::from_le_bytes(bytes),
                None => self.arm9_read_word(address),
            }
        } else {
            self.arm7_read_word(address)
        }
    }

    pub fn read_halfword(&self, address: u32, cpu_type: CpuType) -> u16 {
        if cpu_type == CpuType::Arm9 {
            match self.arm9_cp15.tcm_read::<2>(address) {
                Some(bytes) => u16::from_le_bytes(bytes),
                None => self.arm9_read_halfword(address),
            }
        } else {
            self.arm7_read_halfword(address)
        }
    }

    pub fn read_byte(&self, address: u32, cpu_type: CpuType) -> u8 {
        if cpu_type == CpuType::Arm9 {
            match self.arm9_cp15.tcm_read::<1>(address) {
                Some([byte]) => byte,
                None => self.arm9_read_byte(address),
            }
        } else {
            self.arm7_read_byte(address)
        }
    }
}
