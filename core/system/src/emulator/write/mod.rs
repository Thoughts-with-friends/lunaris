mod write_arm7;
mod write_arm9;

use ds_cpu::arm_cpu::CpuType;

use crate::{
    emulator::Emulator,
    slot2::{Slot2Bus, Width},
};

/// Folds main RAM mirrors together so a watchpoint matches any of them.
const fn watch_key(address: u32) -> u32 {
    if address >> 24 == 0x02 {
        // Main RAM
        address & 0x023F_FFFC // 4MB Main RAM, word aligned
    } else {
        address & !3
    }
}

impl Emulator {
    /// Prints a CPU write that hits `Config::watch_addr` (from check_watch,
    /// stderr "WATCH armN pc~... writeN [addr] = value").
    #[cold]
    fn log_watched_write(&self, address: u32, value: u32, size: u32, cpu_type: CpuType) {
        let cpu = self.get_cpu(cpu_type);
        let pc = cpu.get_pc().wrapping_sub(if cpu.cpsr.thumb_on { 4 } else { 8 });
        eprintln!(
            "WATCH arm{} pc~{pc:08X} write{size} [{address:08X}] = {value:08X}",
            if cpu_type == CpuType::Arm9 { 9 } else { 7 },
        );
    }

    /// Debug watchpoint: called first by write_word/halfword/byte.
    fn check_watch(&self, address: u32, value: u32, size: u32, cpu_type: CpuType) {
        if let Some(watch) = self.config.watch_addr
            && watch_key(address) == watch_key(watch)
        {
            self.log_watched_write(address, value, size, cpu_type);
        }
    }

    /// CPU-side 32-bit store: the entry point for every data write.
    ///
    /// ```text
    /// from: STR/STM/PUSH/SWP handlers, HLE BIOS (Caller::write32)
    ///   |
    ///   v
    /// write_word(address, word, cpu)
    ///   1. check_watch()                     debug watchpoint
    ///   2. ARM9: CP15 TCM hit? -> TCM, else -> arm9_write_word()
    ///      ARM7:                            -> arm7_write_word()
    ///   |
    ///   v
    /// to:   RAM / VRAM or a device register; register writes can start
    ///       DMAs, raise IRQs (IPC), feed the GX FIFO, clock SPI...
    /// ```
    pub fn write_word(&mut self, address: u32, word: u32, cpu_type: CpuType) {
        self.sync_before_io(address, cpu_type);
        if Slot2Bus::decodes(address)
            && self.owns_slot2(cpu_type)
            && self.slot2.write(address, word, Width::Word)
        {
            return;
        }
        self.check_watch(address, word, 32, cpu_type);
        if cpu_type == CpuType::Arm9 {
            if !self.arm9_cp15.tcm_write(address, &word.to_le_bytes()) {
                self.arm9_write_word(address, word);
            }
        } else {
            self.arm7_write_word(address, word);
        }
    }

    /// CPU-side 16-bit store (STRH); same routing as [`Self::write_word`]
    /// via arm9_/arm7_write_halfword().
    pub fn write_halfword(&mut self, address: u32, halfword: u16, cpu_type: CpuType) {
        self.sync_before_io(address, cpu_type);
        if Slot2Bus::decodes(address)
            && self.owns_slot2(cpu_type)
            && self.slot2.write(address, halfword.into(), Width::Half)
        {
            return;
        }
        self.check_watch(address, halfword.into(), 16, cpu_type);
        if cpu_type == CpuType::Arm9 {
            if !self.arm9_cp15.tcm_write(address, &halfword.to_le_bytes()) {
                self.arm9_write_halfword(address, halfword);
            }
        } else {
            self.arm7_write_halfword(address, halfword);
        }
    }

    /// CPU-side 8-bit store (STRB); same routing as [`Self::write_word`]
    /// via arm9_/arm7_write_byte().
    pub fn write_byte(&mut self, address: u32, byte: u8, cpu_type: CpuType) {
        self.sync_before_io(address, cpu_type);
        if Slot2Bus::decodes(address)
            && self.owns_slot2(cpu_type)
            && self.slot2.write(address, byte.into(), Width::Byte)
        {
            return;
        }
        self.check_watch(address, byte.into(), 8, cpu_type);
        if cpu_type == CpuType::Arm9 {
            if !self.arm9_cp15.tcm_write(address, &[byte]) {
                self.arm9_write_byte(address, byte);
            }
        } else {
            self.arm7_write_byte(address, byte);
        }
    }
}
