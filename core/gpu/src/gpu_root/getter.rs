// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
use crate::gpu_root::Gpu;

impl Gpu {
    pub fn get_palette(&mut self, engine_a: bool) -> &mut Vec<u8> {
        if engine_a {
            return &mut self.palette_upper;
        }
        &mut self.palette_lower
    }

    /// Get VRAM bank A-D (`id` 0-3) as halfwords (display capture / VRAM
    /// display mode).
    /// # Panics
    /// Panics if `id` is not 0..=3.
    pub fn get_vram_block(&self, id: i32) -> &[u16] {
        assert!((0..4).contains(&id), "Invalid VRAM bank ID: {id}");
        let bytes = self.vram.bank(id as usize);
        // SAFETY: banks are heap allocations of even length; u16 has no
        // invalid bit patterns and the allocator aligns to at least 8.
        unsafe { core::slice::from_raw_parts(bytes.as_ptr().cast::<u16>(), bytes.len() / 2) }
    }

    /// Mutable variant of [`Self::get_vram_block`].
    /// # Panics
    /// Panics if `id` is not 0..=3.
    pub fn get_vram_block_mut(&mut self, id: i32) -> &mut [u16] {
        assert!((0..4).contains(&id), "Invalid VRAM bank ID: {id}");
        let bytes = self.vram.bank_mut(id as usize);
        // SAFETY: see `get_vram_block`.
        unsafe {
            core::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast::<u16>(), bytes.len() / 2)
        }
    }

    pub fn get_dispcnt_a(&self) -> u32 {
        self.engine_upper.get_dispcnt()
    }

    pub fn get_dispcnt_b(&self) -> u32 {
        self.engine_lower.get_dispcnt()
    }

    /// Get DISPSTAT7 register value
    pub fn get_dispstat7(&self) -> u16 {
        self.display_status_arm7.get()
    }

    /// Get DISPSTAT9 register value
    pub fn get_dispstat9(&self) -> u16 {
        self.display_status_arm9.get()
    }

    /// Get BGCNT
    pub fn get_bgcnt_a(&self, index: usize) -> u16 {
        self.engine_upper.get_bgcnt(index)
    }

    pub fn get_bgcnt_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgcnt(index)
    }

    /// Get VCOUNT register value
    pub fn get_vcount(&self) -> u16 {
        self.vertical_count
    }

    /// Get BGH
    pub fn get_bghofs_a(&self, index: usize) -> u16 {
        self.engine_upper.get_bgvofs(index)
    }

    pub fn get_bgvofs_a(&self, index: usize) -> u16 {
        self.engine_upper.get_bgvofs(index)
    }

    pub fn get_bghofs_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgcnt(index)
    }

    pub fn get_bgvofs_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgvofs(index)
    }

    pub fn get_win0v_a(&self) -> u16 {
        self.engine_upper.get_win0v()
    }

    pub fn get_win1v_a(&self) -> u16 {
        self.engine_upper.get_win1v()
    }

    pub fn get_win0v_b(&self) -> u16 {
        self.engine_lower.get_win0v()
    }

    pub fn get_win1v_b(&self) -> u16 {
        self.engine_lower.get_win1v()
    }

    pub fn get_winin_a(&self) -> u16 {
        self.engine_upper.get_winin()
    }

    pub fn get_winin_b(&self) -> u16 {
        self.engine_lower.get_winin()
    }

    pub fn get_winout_a(&self) -> u16 {
        self.engine_upper.get_winout()
    }

    pub fn get_winout_b(&self) -> u16 {
        self.engine_lower.get_winout()
    }

    pub fn get_bldcnt_a(&self) -> u16 {
        self.engine_upper.get_bldcnt()
    }

    pub fn get_bldcnt_b(&self) -> u16 {
        self.engine_lower.get_bldcnt()
    }

    pub fn get_bldalpha_a(&self) -> u16 {
        self.engine_upper.get_bldalpha()
    }

    pub fn get_bldalpha_b(&self) -> u16 {
        self.engine_lower.get_bldalpha()
    }

    pub fn get_disp3dcnt(&self) -> u16 {
        self.engine_3d.get_disp3dcnt()
    }

    pub fn get_master_bright_a(&self) -> u16 {
        self.engine_upper.get_master_bright()
    }

    pub fn get_master_bright_b(&self) -> u16 {
        self.engine_lower.get_master_bright()
    }

    /// Replace uint32_t get_DISPCAPCNT();
    pub fn get_dispcapcnt_a(&self) -> u32 {
        let is_engine_a = true;
        self.engine_upper.get_dispcapcnt(is_engine_a)
    }

    pub fn get_vramstat(&self) -> u8 {
        self.vram.vramstat()
    }

    /// Get VRAM bank configuration A
    pub fn get_vramcnt_a(&self) -> u8 {
        self.vram.cnt(0)
    }

    /// Get VRAM bank configuration B
    pub fn get_vramcnt_b(&self) -> u8 {
        self.vram.cnt(1)
    }

    /// Get VRAM bank configuration C
    pub fn get_vramcnt_c(&self) -> u8 {
        self.vram.cnt(2)
    }

    /// Get VRAM bank configuration D
    pub fn get_vramcnt_d(&self) -> u8 {
        self.vram.cnt(3)
    }

    /// Get VRAM bank configuration E
    pub fn get_vramcnt_e(&self) -> u8 {
        self.vram.cnt(4)
    }

    /// Get VRAM bank configuration F
    pub fn get_vramcnt_f(&self) -> u8 {
        self.vram.cnt(5)
    }

    /// Get VRAM bank configuration G
    pub fn get_vramcnt_g(&self) -> u8 {
        self.vram.cnt(6)
    }

    /// Get VRAM bank configuration H
    pub fn get_vramcnt_h(&self) -> u8 {
        self.vram.cnt(7)
    }

    /// Get VRAM bank configuration I
    pub fn get_vramcnt_i(&self) -> u8 {
        self.vram.cnt(8)
    }

    /// Get POWCNT1 register value
    pub fn get_powcnt1(&self) -> u16 {
        self.power_control_reg.get()
    }

    pub fn get_gxstat(&self) -> u32 {
        self.engine_3d.get_gxstat()
    }

    pub fn get_vert_count(&self) -> u16 {
        self.engine_3d.get_vert_count()
    }

    pub fn get_poly_count(&self) -> u16 {
        self.engine_3d.get_poly_count()
    }
}
