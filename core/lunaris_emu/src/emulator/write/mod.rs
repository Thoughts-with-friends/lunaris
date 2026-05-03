use lunaris_ds_mem_const::{DTCM_MASK, ITCM_MASK};

use crate::cpu::arm_cpu::CpuType;
use crate::emulator::Emulator;

impl Emulator {
    pub fn write_word(&mut self, address: u32, word: u32, cpu_type: CpuType) {
        // 67109048(0x04_000_0b8), word: 2168455169
        if (1351000..1352000).contains(&self.get_timestamp()) {
            #[cfg(feature = "tracing")]
            tracing::debug!(
                "[ARM_CPU::write_word] address: {:08X}, word: {}",
                address,
                word
            );
        }

        if cpu_type == CpuType::Arm9 {
            // NOTE: inline CP15::write_word
            let dtcm_size = self.arm9_cp15.dtcm_size;
            let dtcm_base = self.arm9_cp15.dtcm_base;

            if address < self.arm9_cp15.itcm_size {
                self.arm9_cp15.write_word(address & ITCM_MASK, word);
            } else if address >= dtcm_base && address < (dtcm_base + dtcm_size) {
                self.arm9_cp15.write_word(address & DTCM_MASK, word);
            } else {
                #[cfg(feature = "tracing")]
                if address < 0x04000000 {
                    tracing::warn!("invalid address: {address:#x} < 0x04000000");
                }
                self.arm9_write_word(address, word);
            }
        } else {
            self.arm7_write_word(address, word)
        }
    }

    pub fn write_halfword(&mut self, address: u32, halfword: u16, cpu_type: CpuType) {
        if cpu_type == CpuType::Arm9 {
            // NOTE: inline CP15::write_halfword
            let dtcm_size = self.arm9_cp15.dtcm_size;
            let dtcm_base = self.arm9_cp15.dtcm_base;

            if address < self.arm9_cp15.itcm_size {
                self.arm9_cp15.write_halfword(address & ITCM_MASK, halfword);
            } else if address >= dtcm_base && address < (dtcm_base + dtcm_size) {
                self.arm9_cp15.write_halfword(address & DTCM_MASK, halfword);
            } else {
                self.arm9_write_halfword(address, halfword);
            }
        } else {
            self.arm7_write_halfword(address, halfword)
        }
    }

    pub fn write_byte(&mut self, address: u32, byte: u8, cpu_type: CpuType) {
        if cpu_type == CpuType::Arm9 {
            // NOTE: inline CP15::write_byte
            let dtcm_size = self.arm9_cp15.dtcm_size;
            let dtcm_base = self.arm9_cp15.dtcm_base;

            if address < self.arm9_cp15.itcm_size {
                self.arm9_cp15.write_byte(address & ITCM_MASK, byte as u32);
            } else if address >= dtcm_base && address < (dtcm_base + dtcm_size) {
                self.arm9_cp15.write_byte(address & DTCM_MASK, byte as u32);
            } else {
                self.arm9_write_byte(address, byte);
            }
        } else {
            self.arm7_write_byte(address, byte)
        }
    }
}
