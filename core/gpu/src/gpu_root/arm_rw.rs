// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
use crate::gpu_root::Gpu;

const OAM_MASK: u32 = 0x7FF;

impl Gpu {
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
