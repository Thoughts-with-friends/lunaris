//! cpu.cpp:157
mod read_arm7;
mod read_arm9;

use ds_cpu::arm_cpu::CpuType;
use ds_mem_addr::{ARM7_WRAM_MASK, ARM7_WRAM_START};

use crate::{
    emulator::Emulator,
    slot2::{Slot2Bus, Width},
};

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
        if let Some(value) = self.slot2_read(address, Width::Word, cpu_type) {
            return value;
        }
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
        if let Some(value) = self.slot2_read(address, Width::Half, cpu_type) {
            return value as u16;
        }
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
        if let Some(value) = self.slot2_read(address, Width::Byte, cpu_type) {
            return value as u8;
        }
        if cpu_type == CpuType::Arm9 {
            match self.arm9_cp15.tcm_read::<1>(address) {
                Some([byte]) => byte,
                None => self.arm9_read_byte(address),
            }
        } else {
            self.arm7_read_byte(address)
        }
    }

    /// Whether EXMEMCNT bit 7 gives the GBA slot to `cpu_type` (0 = ARM9,
    /// 1 = ARM7). GBATEK "DS Memory Control - Cartridges and Main RAM".
    pub fn owns_slot2(&self, cpu_type: CpuType) -> bool {
        let arm7_owns = self.ex_mem_cnt & (1 << 7) != 0;
        arm7_owns == (cpu_type == CpuType::Arm7)
    }

    /// A CPU read routed to the Slot-2 device; `None`: not the device's
    /// (no device, other address or the other CPU owns the slot).
    fn slot2_read(&mut self, address: u32, width: Width, cpu_type: CpuType) -> Option<u32> {
        if !Slot2Bus::decodes(address) || !self.owns_slot2(cpu_type) {
            return None;
        }
        self.slot2.read(address, width)
    }
}
