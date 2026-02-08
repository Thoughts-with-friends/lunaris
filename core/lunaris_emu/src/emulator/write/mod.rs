use crate::cpu::arm_cpu::CpuType;
use crate::emulator::Emulator;

impl Emulator {
    pub fn write_word(&mut self, address: u32, word: u32, cpu_type: CpuType) {
        if self.get_cpu(cpu_type).cpu_id <= 0 {
            self.arm9_cp15.write_word(address, word)
        } else {
            self.arm7_write_word(address, word)
        }
    }

    pub fn write_halfword(&mut self, address: u32, halfword: u16, cpu_type: CpuType) {
        if self.get_cpu(cpu_type).cpu_id <= 0 {
            self.arm9_cp15.write_halfword(address, halfword)
        } else {
            self.arm7_write_halfword(address, halfword)
        }
    }

    pub fn write_byte(&mut self, address: u32, byte: u8, cpu_type: CpuType) {
        if self.get_cpu(cpu_type).cpu_id <= 0 {
            self.arm9_cp15.write_byte(address, byte as u32)
        } else {
            self.arm7_write_byte(address, byte)
        }
    }
}
