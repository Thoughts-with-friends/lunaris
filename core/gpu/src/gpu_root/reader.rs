// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
use crate::gpu_root::Gpu;
use lunaris_ds_mem_const::*;

impl Gpu {
    // text image
    pub fn read_teximage_u8(&self, address: u32) -> u8 {
        let mut reg = 0;

        // VRAM A-D
        if self.vramcnt_a.enabled {
            let offset: u32 = self.vramcnt_a.offset * VRAM_A_SIZE;
            if addr_in_range(address, offset, VRAM_A_SIZE) && self.vramcnt_a.mst == 3 {
                let index = (address & VRAM_A_MASK) as usize;
                reg |= self.vram_a[index];
            }
        }

        if self.vramcnt_b.enabled {
            let offset: u32 = self.vramcnt_b.offset * VRAM_A_SIZE;
            if addr_in_range(address, offset, VRAM_B_SIZE) && self.vramcnt_b.mst == 3 {
                let index = (address & VRAM_B_MASK) as usize;
                reg |= self.vram_b[index];
            }
        }

        if self.vramcnt_c.enabled {
            let offset: u32 = self.vramcnt_c.offset * VRAM_A_SIZE;
            if addr_in_range(address, offset, VRAM_C_SIZE) && self.vramcnt_c.mst == 3 {
                let index = (address & VRAM_C_MASK) as usize;
                reg |= self.vram_c[index];
            }
        }

        if self.vramcnt_d.enabled {
            let offset: u32 = self.vramcnt_d.offset * VRAM_A_SIZE;
            if addr_in_range(address, offset, VRAM_D_SIZE) && self.vramcnt_d.mst == 3 {
                let index = (address & VRAM_D_MASK) as usize;
                reg |= self.vram_d[index];
            }
        }

        reg
    }

    pub fn read_teximage_u16(&self, address: u32) -> u16 {
        self.read_teximage_u8(address) as u16
    }

    // textpal
    pub fn read_texpal_u16(&self, address: u32) -> u16 {
        let mut reg = 0;
        // let mut lorp = false;

        // VRAM E-G
        if self.vramcnt_e.enabled
            && addr_in_range(address, 0, VRAM_E_SIZE)
            && self.vramcnt_e.mst == 3
        {
            let index = (address & VRAM_E_MASK) as usize;
            reg |= self.vram_e[index];
            // lorp = true;
        }

        if self.vramcnt_f.enabled {
            let mut addr = (self.vramcnt_f.offset & 0x1) + (self.vramcnt_f.offset & 0x2) * 2;
            addr *= VRAM_F_SIZE;
            if addr_in_range(address, addr, VRAM_F_SIZE) && self.vramcnt_f.mst == 3 {
                let index = (address & VRAM_F_MASK) as usize;
                reg |= self.vram_f[index];
                // lorp = true;
            }
        }

        if self.vramcnt_g.enabled {
            let mut addr = (self.vramcnt_g.offset & 0x1) + (self.vramcnt_g.offset & 0x2) * 2;
            addr *= VRAM_G_SIZE;
            if addr_in_range(address, addr, VRAM_G_SIZE) && self.vramcnt_g.mst == 3 {
                let index = (address & VRAM_G_MASK) as usize;
                reg |= self.vram_g[index];
                // lorp = true;
            }
        }

        reg as u16
    }

    pub fn read_texpal_u32(&self, address: u32) -> u32 {
        self.read_texpal_u16(address) as u32
    }

    // LCDC
    pub fn read_lcdc_u8(&self, address: u32) -> u8 {
        let mut reg = 0;

        // VRAM A-I
        if self.vramcnt_a.enabled
            && addr_in_range(address, VRAM_LCDC_A, VRAM_A_SIZE)
            && self.vramcnt_a.mst == 0
        {
            let index = (address & VRAM_A_MASK) as usize;
            reg |= self.vram_a[index];
        }

        if self.vramcnt_b.enabled
            && addr_in_range(address, VRAM_LCDC_B, VRAM_B_SIZE)
            && self.vramcnt_b.mst == 0
        {
            let index = (address & VRAM_B_MASK) as usize;
            reg |= self.vram_b[index];
        }

        if self.vramcnt_c.enabled
            && addr_in_range(address, VRAM_LCDC_C, VRAM_C_SIZE)
            && self.vramcnt_c.mst == 0
        {
            let index = (address & VRAM_C_MASK) as usize;
            reg |= self.vram_c[index];
        }

        if self.vramcnt_d.enabled
            && addr_in_range(address, VRAM_LCDC_D, VRAM_D_SIZE)
            && self.vramcnt_d.mst == 0
        {
            let index = (address & VRAM_D_MASK) as usize;
            reg |= self.vram_d[index];
        }

        if self.vramcnt_e.enabled
            && addr_in_range(address, VRAM_LCDC_E, VRAM_E_SIZE)
            && self.vramcnt_e.mst == 0
        {
            let index = (address & VRAM_E_MASK) as usize;
            reg |= self.vram_e[index];
        }

        if self.vramcnt_f.enabled
            && addr_in_range(address, VRAM_LCDC_F, VRAM_F_SIZE)
            && self.vramcnt_f.mst == 0
        {
            let index = (address & VRAM_F_MASK) as usize;
            reg |= self.vram_f[index];
        }

        if self.vramcnt_g.enabled
            && addr_in_range(address, VRAM_LCDC_G, VRAM_G_SIZE)
            && self.vramcnt_g.mst == 0
        {
            let index = (address & VRAM_G_MASK) as usize;
            reg |= self.vram_g[index];
        }

        if self.vramcnt_h.enabled
            && addr_in_range(address, VRAM_LCDC_H, VRAM_H_SIZE)
            && self.vramcnt_h.mst == 0
        {
            let index = (address & VRAM_H_MASK) as usize;
            reg |= self.vram_h[index];
        }

        if self.vramcnt_i.enabled
            && addr_in_range(address, VRAM_LCDC_I, VRAM_I_SIZE)
            && self.vramcnt_i.mst == 0
        {
            let index = (address & VRAM_I_MASK) as usize;
            reg |= self.vram_i[index];
        }

        reg
    }

    pub fn read_lcdc_u16(&self, address: u32) -> u16 {
        self.read_lcdc_u8(address) as u16
    }

    pub fn read_lcdc_u32(&self, address: u32) -> u32 {
        self.read_lcdc_u8(address) as u32
    }

    // moved arm_rw.rs
    // pub fn read_oam<T>(&self, address: u32)
}
