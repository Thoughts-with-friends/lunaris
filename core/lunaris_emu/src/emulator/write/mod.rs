mod write_arm7;
mod write_arm9;

use crate::cpu::arm_cpu::CpuType;
use crate::emulator::Emulator;

impl Emulator {
    pub fn write_word(&mut self, address: u32, word: u32, cpu_type: CpuType) {
        if cpu_type == CpuType::Arm9 {
            if !self.arm9_cp15.tcm_write(address, &word.to_le_bytes()) {
                self.arm9_write_word(address, word);
            }
        } else {
            self.arm7_write_word(address, word);
        }
    }

    pub fn write_halfword(&mut self, address: u32, halfword: u16, cpu_type: CpuType) {
        if cpu_type == CpuType::Arm9 {
            if !self.arm9_cp15.tcm_write(address, &halfword.to_le_bytes()) {
                self.arm9_write_halfword(address, halfword);
            }
        } else {
            self.arm7_write_halfword(address, halfword);
        }
    }

    pub fn write_byte(&mut self, address: u32, byte: u8, cpu_type: CpuType) {
        if cpu_type == CpuType::Arm9 {
            if !self.arm9_cp15.tcm_write(address, &[byte]) {
                self.arm9_write_byte(address, byte);
            }
        } else {
            self.arm7_write_byte(address, byte);
        }
    }
}
