//! cpu.cpp:157
use lunaris_ds_mem_const::{DTCM_MASK, ITCM_MASK};

use crate::cpu::arm_cpu::CpuType;
use crate::emulator::Emulator;

impl Emulator {
    pub fn read_word(&mut self, address: u32, cpu_type: CpuType) -> u32 {
        if cpu_type == CpuType::Arm9 {
            // NOTE: inline CP15::read_word
            let dtcm_size = self.arm9_cp15.dtcm_size;
            let dtcm_base = self.arm9_cp15.dtcm_base;
            let dtcm_write_only = self.arm9_cp15.dtcm_write_only();

            tracing::debug!(%dtcm_size, %dtcm_base, %self.arm9_cp15.itcm_size, %address, %dtcm_write_only);

            if address < self.arm9_cp15.itcm_size {
                self.arm9_cp15.read_word(address & ITCM_MASK)
            } else if address >= dtcm_base
                && address < (dtcm_base + dtcm_size)
                && !self.arm9_cp15.dtcm_write_only()
            {
                //             0x2000800
                #[cfg(feature = "tracing")]
                if address == 0x2076EC0 {
                    tracing::debug!("DERP")
                }

                self.arm9_cp15.read_word(address & DTCM_MASK)
            } else {
                #[cfg(feature = "tracing")]
                if address < 0x2000000 {
                    tracing::warn!("invalid address: {address:#x} < 0x2000000");
                }

                let word = self.arm9_read_word(address);

                #[cfg(feature = "tracing")]
                tracing::debug!(%self.system_timestamp, %address, %word);

                word
            }
        } else {
            self.arm7_read_word(address)
        }
    }

    pub fn read_halfword(&self, address: u32, cpu_type: CpuType) -> u16 {
        if cpu_type == CpuType::Arm9 {
            // NOTE: inline CP15::read_halfword
            let dtcm_size = self.arm9_cp15.dtcm_size;
            let dtcm_base = self.arm9_cp15.dtcm_base;

            if address < self.arm9_cp15.itcm_size {
                self.arm9_cp15.read_halfword(address & ITCM_MASK)
            } else if address >= dtcm_base && address < (dtcm_base + dtcm_size) {
                self.arm9_cp15.read_halfword(address & DTCM_MASK)
            } else {
                self.arm9_read_halfword(address)
            }
        } else {
            self.arm7_read_halfword(address)
        }
    }

    pub fn read_byte(&self, address: u32, cpu_type: CpuType) -> u8 {
        if cpu_type == CpuType::Arm9 {
            // NOTE: inline CP15::read_byte
            let dtcm_size = self.arm9_cp15.dtcm_size;
            let dtcm_base = self.arm9_cp15.dtcm_base;

            if address < self.arm9_cp15.itcm_size {
                self.arm9_cp15.read_byte(address & ITCM_MASK)
            } else if address >= dtcm_base && address < (dtcm_base + dtcm_size) {
                self.arm9_cp15.read_byte(address & DTCM_MASK)
            } else {
                self.arm9_read_byte(address)
            }
        } else {
            self.arm7_read_byte(address)
        }
    }
}
