// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
use crate::gpu_root::Gpu;
use lunaris_ds_mem_const::*;

const OAM_MASK: u32 = 0x7FF;

impl Gpu {
    pub fn read_arm7_u16(&self, address: u32) -> u16 {
        {
            let mut reg = 0;
            if self.vramcnt_c.enabled {
                if addr_in_range(
                    address,
                    0x06000000 + self.vramcnt_c.offset * 0x20000,
                    VRAM_C_SIZE as u32,
                ) && self.vramcnt_c.mst == 2
                {
                    let start = (address & VRAM_C_MASK) as usize;
                    let end = (start + 2) as usize;
                    reg |= u16::from_le_bytes(self.vram_c[start..end].try_into().unwrap());
                }
            }
            if self.vramcnt_d.enabled {
                if addr_in_range(
                    address,
                    0x06000000 + self.vramcnt_d.offset * 0x20000,
                    VRAM_D_SIZE as u32,
                ) && self.vramcnt_d.mst == 2
                {
                    let start = (address & VRAM_D_MASK) as usize;
                    let end = (start + 2) as usize;
                    reg |= u16::from_le_bytes(self.vram_d[start..end].try_into().unwrap());
                }
            }
            return reg;
        }
    }

    pub fn read_arm7_u32(&self, address: u32) -> u32 {
        {
            let mut reg = 0;
            if self.vramcnt_c.enabled {
                if addr_in_range(
                    address,
                    0x06000000 + self.vramcnt_c.offset * 0x20000,
                    VRAM_C_SIZE as u32,
                ) && self.vramcnt_c.mst == 2
                {
                    let start = (address & VRAM_C_MASK) as usize;
                    let end = (start + 4) as usize;
                    reg |= u32::from_le_bytes(self.vram_c[start..end].try_into().unwrap());
                }
            }
            if self.vramcnt_d.enabled {
                if addr_in_range(
                    address,
                    0x06000000 + self.vramcnt_d.offset * 0x20000,
                    VRAM_D_SIZE as u32,
                ) && self.vramcnt_d.mst == 2
                {
                    let start = (address & VRAM_D_MASK) as usize;
                    let end = (start + 4) as usize;
                    reg |= u32::from_le_bytes(self.vram_d[start..end].try_into().unwrap());
                }
            }
            return reg;
        }
    }

    /// Write 32-bit word from ARM7 to VRAM (C and D)
    pub fn write_arm7_u32(&mut self, address: u32, value: u32) {
        // VRAM C
        if self.vramcnt_c.enabled {
            let start = 0x06000000 + self.vramcnt_c.offset * 0x20000;
            if addr_in_range(address, start, VRAM_C_SIZE as u32) && self.vramcnt_c.mst == 2 {
                let start = (address & VRAM_C_MASK) as usize;
                self.vram_c[start..start + 4].copy_from_slice(&value.to_le_bytes());
            }
        }

        // VRAM D
        if self.vramcnt_d.enabled {
            let start = 0x06000000 + self.vramcnt_d.offset * 0x20000;
            if addr_in_range(address, start, VRAM_D_SIZE as u32) && self.vramcnt_d.mst == 2 {
                let start = (address & VRAM_D_MASK) as usize;
                self.vram_d[start..start + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
    }

    /// Write 8-bit byte from ARM7 to VRAM (C and D)
    /// Matches the behavior of C++: VRAM_C[address & mask] = value;
    pub fn write_arm7_u8(&mut self, address: u32, value: u8) {
        // VRAM C
        if self.vramcnt_c.enabled {
            let start = 0x06000000 + self.vramcnt_c.offset * 0x20000;
            if addr_in_range(address, start, VRAM_C_SIZE as u32) && self.vramcnt_c.mst == 2 {
                let idx = (address & VRAM_C_MASK) as usize;
                self.vram_c[idx] = value;
            }
        }

        // VRAM D
        if self.vramcnt_d.enabled {
            let start = 0x06000000 + self.vramcnt_d.offset * 0x20000;
            if addr_in_range(address, start, VRAM_D_SIZE) && self.vramcnt_d.mst == 2 {
                let idx = (address & VRAM_D_MASK) as usize;
                self.vram_d[idx] = value;
            }
        }
    }

    pub fn read_oam_u8(&self, address: u32) -> u8 {
        let index = (address & OAM_MASK) as usize;
        self.oam[index]
    }

    pub fn read_oam_u16(&self, address: u32) -> u16 {
        let idx = (address & OAM_MASK) as usize;

        let lo = self.oam[idx];
        let hi = self.oam[(idx + 1) & OAM_MASK as usize];

        u16::from_le_bytes([lo, hi])
    }

    pub fn read_oam_u32(&self, address: u32) -> u32 {
        let idx = (address & OAM_MASK) as usize;

        let b0 = self.oam[idx];
        let b1 = self.oam[(idx + 1) & OAM_MASK as usize];
        let b2 = self.oam[(idx + 2) & OAM_MASK as usize];
        let b3 = self.oam[(idx + 3) & OAM_MASK as usize];

        u32::from_le_bytes([b0, b1, b2, b3])
    }

    pub fn read_oam_i16(&self, address: u32) -> i16 {
        let idx = (address & OAM_MASK) as usize;

        let lo = self.oam[idx];
        let hi = self.oam[(idx + 1) & OAM_MASK as usize];

        i16::from_le_bytes([lo, hi])
    }
}
