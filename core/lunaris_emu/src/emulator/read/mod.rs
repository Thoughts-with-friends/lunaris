//! cpu.cpp:157
use crate::cpu::arm_cpu::CpuType;
use crate::emulator::Emulator;

impl Emulator {
    pub fn read_word(&mut self, address: u32, cpu_type: CpuType) -> u32 {
        if self.get_cpu(cpu_type).cpu_id <= 0 {
            self.arm9_cp15.read_word(address)
        } else {
            self.arm7_read_word(address)
        }
    }

    pub fn read_halfword(&self, address: u32, cpu_type: CpuType) -> u16 {
        if self.get_cpu(cpu_type).cpu_id <= 0 {
            self.arm9_cp15.read_halfword(address)
        } else {
            self.arm7_read_halfword(address)
        }
    }

    pub fn read_byte(&self, address: u32, cpu_type: CpuType) -> u8 {
        if self.get_cpu(cpu_type).cpu_id <= 0 {
            self.arm9_cp15.read_byte(address)
        } else {
            self.arm7_read_byte(address)
        }
    }
}
