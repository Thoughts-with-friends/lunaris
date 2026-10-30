//! cpu.cpp:157
mod read_arm7;
mod read_arm9;

use ds_cpu::arm_cpu::CpuType;
use ds_mem_addr::{ARM7_WRAM_MASK, ARM7_WRAM_START};

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
            // DS Memory Control - WRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolwram
            1 => (false, (address & 0x3FFF) as usize), // WRAMCNT 1: ARM7 gets the first 16K
            2 => (false, ((address & 0x3FFF) + 0x4000) as usize), // WRAMCNT 2: ARM7 gets the second 16K
            _ => (false, (address & 0x7FFF) as usize),            // WRAMCNT 3: ARM7 gets all 32K
        }
    }

    /// CPU-side 32-bit load: the entry point for every instruction fetch
    /// and data read of either CPU.
    ///
    /// ```text
    /// from: execute() (ARM fetch), LDR/LDM/POP/SWP handlers,
    ///       HLE BIOS (bios.rs Caller::read32)
    ///   |
    ///   v
    /// read_word(address, cpu)
    ///   ARM9: CP15 ITCM/DTCM hit?  -> TCM contents
    ///         otherwise            -> arm9_read_word()
    ///   ARM7:                      -> arm7_read_word()
    ///   |
    ///   v
    /// to:   RAM / BIOS / VRAM or the I/O register of a device (GPU, DMA,
    ///       timers, IPC, cartridge, SPI, SPU, Wi-Fi...)
    /// ```
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

    /// CPU-side 16-bit load (Thumb fetch, LDRH/LDRSH); same routing as
    /// [`Self::read_word`] via arm9_/arm7_read_halfword().
    pub fn read_halfword(&mut self, address: u32, cpu_type: CpuType) -> u16 {
        if cpu_type == CpuType::Arm9 {
            match self.arm9_cp15.tcm_read::<2>(address) {
                Some(bytes) => u16::from_le_bytes(bytes),
                None => self.arm9_read_halfword(address),
            }
        } else {
            self.arm7_read_halfword(address)
        }
    }

    /// CPU-side 8-bit load (LDRB/LDRSB); same routing as
    /// [`Self::read_word`] via arm9_/arm7_read_byte().
    pub fn read_byte(&mut self, address: u32, cpu_type: CpuType) -> u8 {
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
