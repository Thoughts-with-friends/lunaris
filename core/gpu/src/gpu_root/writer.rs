// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
use crate::gpu_root::Gpu;

impl Gpu {
    /// Write to palette A (`address` may be absolute; only bits 0-9 count).
    pub fn write_palette_a(&mut self, address: u32, value: u16) {
        let index = (address & 0x3FE) as usize;
        self.palette_upper[index..index + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// Write to palette B (`address` may be absolute; only bits 0-9 count).
    pub fn write_palette_b(&mut self, address: u32, value: u16) {
        let index = (address & 0x3FE) as usize;
        self.palette_lower[index..index + 2].copy_from_slice(&value.to_le_bytes());
    }

    pub fn write_oam(&mut self, address: u32, halfword: u16) {
        let index = (address & 0x7FE) as usize;
        self.oam[index..index + 2].copy_from_slice(&halfword.to_le_bytes());
    }
}
