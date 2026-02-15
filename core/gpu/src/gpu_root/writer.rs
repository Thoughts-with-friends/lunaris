// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
use crate::gpu_root::Gpu;
use lunaris_ds_mem_const::*;

impl Gpu {
    /// Write to palette A
    pub fn write_palette_a(&mut self, address: u32, value: u16) {
        let address = address as usize;
        if address < self.palette_upper.len() {
            self.palette_upper[address] = (value & 0xFF) as u8;
        }
        if address + 1 < self.palette_upper.len() {
            self.palette_upper[address + 1] = ((value >> 8) & 0xFF) as u8;
        }
    }

    /// Write to palette B
    pub fn write_palette_b(&mut self, address: u32, value: u16) {
        let address = address as usize;
        if address < self.palette_lower.len() {
            self.palette_lower[address] = (value & 0xFF) as u8;
        }
        if address + 1 < self.palette_lower.len() {
            self.palette_lower[address + 1] = ((value >> 8) & 0xFF) as u8;
        }
    }

    pub fn write_bga(&mut self, address: u32, halfword: u16) {
        #[cfg(feature = "tracing")]
        tracing::info!("BGA: Write to {} of {}", address, halfword);

        // VRAM A-G
        if self.vramcnt_a.enabled
            && addr_in_range(
                address,
                VRAM_BGA_START + (self.vramcnt_a.offset * 0x20000),
                VRAM_A_SIZE,
            )
            && self.vramcnt_a.mst == 1
        {
            let index = (address & VRAM_A_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_a[index] = bytes[0];
            self.vram_a[index + 1] = bytes[1];
        }

        if self.vramcnt_b.enabled
            && addr_in_range(
                address,
                VRAM_BGA_START + (self.vramcnt_b.offset * 0x20000),
                VRAM_B_SIZE,
            )
            && self.vramcnt_b.mst == 1
        {
            let index = (address & VRAM_B_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_b[index] = bytes[0];
            self.vram_b[index + 1] = bytes[1];
        }

        if self.vramcnt_c.enabled
            && addr_in_range(
                address,
                VRAM_BGA_START + (self.vramcnt_c.offset * 0x20000),
                VRAM_C_SIZE,
            )
            && self.vramcnt_c.mst == 1
        {
            let index = (address & VRAM_C_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_c[index] = bytes[0];
            self.vram_c[index + 1] = bytes[1];
        }

        if self.vramcnt_d.enabled
            && addr_in_range(
                address,
                VRAM_BGA_START + (self.vramcnt_d.offset * 0x20000),
                VRAM_D_SIZE,
            )
            && self.vramcnt_d.mst == 1
        {
            let index = (address & VRAM_D_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_d[index] = bytes[0];
            self.vram_d[index + 1] = bytes[1];
        }

        if self.vramcnt_e.enabled
            && addr_in_range(address, VRAM_BGA_START, VRAM_E_SIZE)
            && self.vramcnt_e.mst == 1
        {
            let index = (address & VRAM_E_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_e[index] = bytes[0];
            self.vram_e[index + 1] = bytes[1];
        }

        let f_offset =
            (self.vramcnt_f.offset & 0x1) * 0x4000 + (self.vramcnt_f.offset & 0x2) * 0x10000;

        if self.vramcnt_f.enabled
            && addr_in_range(address, VRAM_BGA_START, f_offset)
            && self.vramcnt_f.mst == 1
        {
            let index = (address & VRAM_F_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_f[index] = bytes[0];
            self.vram_f[index + 1] = bytes[1];
        }

        let g_offset =
            (self.vramcnt_g.offset & 0x1) * 0x4000 + (self.vramcnt_g.offset & 0x2) * 0x10000;

        if self.vramcnt_g.enabled
            && addr_in_range(address, VRAM_BGA_START, g_offset)
            && self.vramcnt_g.mst == 1
        {
            let index = (address & VRAM_G_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_g[index] = bytes[0];
            self.vram_g[index + 1] = bytes[1];
        }
    }

    pub fn write_bgb(&mut self, address: u32, halfword: u16) {
        #[cfg(feature = "tracing")]
        tracing::info!("BGB: Write to {} of {}", address, halfword);

        // VRAM C, H and I
        // NOTE: Why is only this self.vramcnt_c.mst value 4?
        if self.vramcnt_c.enabled
            && addr_in_range(address, VRAM_BGB_C, VRAM_C_SIZE)
            && self.vramcnt_c.mst == 4
        {
            let index = (address & VRAM_C_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_c[index] = bytes[0];
            self.vram_c[index + 1] = bytes[1];
        }

        if self.vramcnt_h.enabled
            && addr_in_range(address, VRAM_BGB_H, VRAM_H_SIZE)
            && self.vramcnt_h.mst == 1
        {
            let index = (address & VRAM_H_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_h[index] = bytes[0];
            self.vram_h[index + 1] = bytes[1];
        }

        if self.vramcnt_i.enabled
            && addr_in_range(address, VRAM_BGB_I, VRAM_I_SIZE)
            && self.vramcnt_i.mst == 1
        {
            let index = (address & VRAM_I_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_i[index] = bytes[0];
            self.vram_i[index + 1] = bytes[1];
        }
    }

    pub fn write_obja(&mut self, address: u32, halfword: u16) {
        #[cfg(feature = "tracing")]
        tracing::info!("OBJA WRITE: {}, {}", address, halfword);

        // VRAM A-G
        if self.vramcnt_a.enabled
            && addr_in_range(
                address,
                VRAM_OBJA_START + ((self.vramcnt_a.offset & 0x1) * 0x20000),
                VRAM_A_SIZE,
            )
            && self.vramcnt_a.mst == 2
        {
            let index = (address & VRAM_H_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_a[index] = bytes[0];
            self.vram_a[index + 1] = bytes[1];
        }

        if self.vramcnt_b.enabled
            && addr_in_range(
                address,
                VRAM_OBJA_START + ((self.vramcnt_b.offset & 0x1) * 0x20000),
                VRAM_A_SIZE,
            )
            && self.vramcnt_b.mst == 2
        {
            let index = (address & VRAM_H_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_b[index] = bytes[0];
            self.vram_b[index + 1] = bytes[1];
        }

        if self.vramcnt_e.enabled
            && addr_in_range(address, VRAM_OBJA_START, VRAM_E_SIZE)
            && self.vramcnt_e.mst == 1
        {
            let index = (address & VRAM_E_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_e[index] = bytes[0];
            self.vram_e[index + 1] = bytes[1];
        }

        let f_offset =
            (self.vramcnt_f.offset & 0x1) * 0x4000 + (self.vramcnt_f.offset & 0x2) * 0x10000;

        if self.vramcnt_f.enabled
            && addr_in_range(address, VRAM_OBJA_START, f_offset)
            && self.vramcnt_f.mst == 2
        {
            let index = (address & VRAM_G_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_g[index] = bytes[0];
            self.vram_g[index + 1] = bytes[1];
        }

        let g_offset =
            (self.vramcnt_g.offset & 0x1) * 0x4000 + (self.vramcnt_g.offset & 0x2) * 0x10000;

        if self.vramcnt_g.enabled
            && addr_in_range(address, VRAM_OBJA_START, g_offset)
            && self.vramcnt_g.mst == 2
        {
            let index = (address & VRAM_G_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_g[index] = bytes[0];
            self.vram_g[index + 1] = bytes[1];
        }
    }

    pub fn write_objb(&mut self, address: u32, halfword: u16) {
        #[cfg(feature = "tracing")]
        tracing::info!("OBJB WRITE: {}, {}", address, halfword);

        // VRAM D and I
        if self.vramcnt_d.enabled
            && addr_in_range(address, VRAM_OBJB_START, VRAM_D_SIZE)
            && self.vramcnt_d.mst == 4
        {
            let index = (address & VRAM_D_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_d[index] = bytes[0];
            self.vram_a[index + 1] = bytes[1];
        }

        if self.vramcnt_i.enabled
            && addr_in_range(address, VRAM_OBJB_START, VRAM_I_SIZE)
            && self.vramcnt_i.mst == 2
        {
            let index = (address & VRAM_I_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_i[index] = bytes[0];
            self.vram_i[index + 1] = bytes[1];
        }
    }

    pub fn write_lcdc(&mut self, address: u32, halfword: u16) {
        #[cfg(feature = "tracing")]
        tracing::info!("LCDC WRITE: {}, {}", address, halfword);

        // VRAM A-I
        if self.vramcnt_a.enabled
            && addr_in_range(address, VRAM_LCDC_A, VRAM_A_SIZE)
            && self.vramcnt_a.mst == 0
        {
            let index = (address & VRAM_A_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_a[index] = bytes[0];
            self.vram_a[index + 1] = bytes[1];
        }

        if self.vramcnt_b.enabled
            && addr_in_range(address, VRAM_LCDC_B, VRAM_B_SIZE)
            && self.vramcnt_b.mst == 0
        {
            let index = (address & VRAM_B_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_b[index] = bytes[0];
            self.vram_b[index + 1] = bytes[1];
        }

        if self.vramcnt_c.enabled
            && addr_in_range(address, VRAM_LCDC_C, VRAM_C_SIZE)
            && self.vramcnt_c.mst == 0
        {
            let index = (address & VRAM_C_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_c[index] = bytes[0];
            self.vram_c[index + 1] = bytes[1];
        }

        if self.vramcnt_d.enabled
            && addr_in_range(address, VRAM_LCDC_D, VRAM_D_SIZE)
            && self.vramcnt_d.mst == 0
        {
            let index = (address & VRAM_D_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_d[index] = bytes[0];
            self.vram_d[index + 1] = bytes[1];
        }

        if self.vramcnt_e.enabled
            && addr_in_range(address, VRAM_LCDC_E, VRAM_E_SIZE)
            && self.vramcnt_e.mst == 0
        {
            let index = (address & VRAM_E_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_e[index] = bytes[0];
            self.vram_e[index + 1] = bytes[1];
        }

        if self.vramcnt_f.enabled
            && addr_in_range(address, VRAM_LCDC_F, VRAM_F_SIZE)
            && self.vramcnt_f.mst == 0
        {
            let index = (address & VRAM_F_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_f[index] = bytes[0];
            self.vram_f[index + 1] = bytes[1];
        }

        if self.vramcnt_g.enabled
            && addr_in_range(address, VRAM_LCDC_G, VRAM_G_SIZE)
            && self.vramcnt_g.mst == 0
        {
            let index = (address & VRAM_G_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_g[index] = bytes[0];
            self.vram_g[index + 1] = bytes[1];
        }

        if self.vramcnt_h.enabled
            && addr_in_range(address, VRAM_LCDC_H, VRAM_H_SIZE)
            && self.vramcnt_h.mst == 0
        {
            let index = (address & VRAM_H_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_h[index] = bytes[0];
            self.vram_h[index + 1] = bytes[1];
        }

        if self.vramcnt_i.enabled
            && addr_in_range(address, VRAM_LCDC_I, VRAM_I_SIZE)
            && self.vramcnt_i.mst == 0
        {
            let index = (address & VRAM_I_MASK) as usize;
            let bytes = halfword.to_le_bytes();

            self.vram_i[index] = bytes[0];
            self.vram_i[index + 1] = bytes[1];
        }
    }

    pub fn write_oam(&mut self, address: u32, halfword: u16) {
        let index = (address & 0x7FF) as usize;
        self.oam[index] = halfword as u8;
    }
}
