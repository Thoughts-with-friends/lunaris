// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
use crate::Gpu;

impl Gpu {
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

    /// Reads DISPCNT (display control) of engine A (0x04000000).
    pub fn get_dispcnt_a(&self) -> u32 {
        self.engine_upper.get_dispcnt()
    }

    /// Reads DISPCNT (display control) of engine B (0x04001000).
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

    /// Reads BGxCNT (BG control) of engine B (0x04001000).
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

    /// Reads BGxVOFS (BG Y scroll) of engine A (0x04000000).
    pub fn get_bgvofs_a(&self, index: usize) -> u16 {
        self.engine_upper.get_bgvofs(index)
    }

    /// Reads BGxHOFS (BG X scroll) of engine B (0x04001000).
    pub fn get_bghofs_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgcnt(index)
    }

    /// Reads BGxVOFS (BG Y scroll) of engine B (0x04001000).
    pub fn get_bgvofs_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgvofs(index)
    }

    /// Reads WIN0V (window 0 Y range) of engine A (0x04000000).
    pub fn get_win0v_a(&self) -> u16 {
        self.engine_upper.get_win0v()
    }

    /// Reads WIN1V (window 1 Y range) of engine A (0x04000000).
    pub fn get_win1v_a(&self) -> u16 {
        self.engine_upper.get_win1v()
    }

    /// Reads WIN0V (window 0 Y range) of engine B (0x04001000).
    pub fn get_win0v_b(&self) -> u16 {
        self.engine_lower.get_win0v()
    }

    /// Reads WIN1V (window 1 Y range) of engine B (0x04001000).
    pub fn get_win1v_b(&self) -> u16 {
        self.engine_lower.get_win1v()
    }

    /// Reads WININ (layers inside WIN0/WIN1) of engine A (0x04000000).
    pub fn get_winin_a(&self) -> u16 {
        self.engine_upper.get_winin()
    }

    /// Reads WININ (layers inside WIN0/WIN1) of engine B (0x04001000).
    pub fn get_winin_b(&self) -> u16 {
        self.engine_lower.get_winin()
    }

    /// Reads WINOUT (layers outside / in the OBJ window) of engine A (0x04000000).
    pub fn get_winout_a(&self) -> u16 {
        self.engine_upper.get_winout()
    }

    /// Reads WINOUT (layers outside / in the OBJ window) of engine B (0x04001000).
    pub fn get_winout_b(&self) -> u16 {
        self.engine_lower.get_winout()
    }

    /// Reads BLDCNT (color effect selection) of engine A (0x04000000).
    pub fn get_bldcnt_a(&self) -> u16 {
        self.engine_upper.get_bldcnt()
    }

    /// Reads BLDCNT (color effect selection) of engine B (0x04001000).
    pub fn get_bldcnt_b(&self) -> u16 {
        self.engine_lower.get_bldcnt()
    }

    /// Reads BLDALPHA (alpha blend weights) of engine A (0x04000000).
    pub fn get_bldalpha_a(&self) -> u16 {
        self.engine_upper.get_bldalpha()
    }

    /// Reads BLDALPHA (alpha blend weights) of engine B (0x04001000).
    pub fn get_bldalpha_b(&self) -> u16 {
        self.engine_lower.get_bldalpha()
    }

    /// Reads DISP3DCNT (3D display control).
    pub fn get_disp3dcnt(&self) -> u16 {
        self.engine_3d.get_disp3dcnt()
    }

    /// Reads MASTER_BRIGHT (screen fade) of engine A (0x04000000).
    pub fn get_master_bright_a(&self) -> u16 {
        self.engine_upper.get_master_bright()
    }

    /// Reads MASTER_BRIGHT (screen fade) of engine B (0x04001000).
    pub fn get_master_bright_b(&self) -> u16 {
        self.engine_lower.get_master_bright()
    }

    /// Replace uint32_t get_DISPCAPCNT();
    pub fn get_dispcapcnt_a(&self) -> u32 {
        let is_engine_a = true;
        self.engine_upper.get_dispcapcnt(is_engine_a)
    }

    /// VRAMSTAT (0x04000240 read): which of banks C/D are mapped to the ARM7.
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
}

impl Gpu {
    /// Set upper screen framebuffer
    pub fn set_upper_buffer(&mut self, buffer: Vec<u32>) {
        self.engine_upper.set_framebuffer(buffer);
    }

    /// Set lower screen framebuffer
    pub fn set_lower_buffer(&mut self, buffer: Vec<u32>) {
        self.engine_lower.set_framebuffer(buffer);
    }

    // ===== Setters / Commands =====

    /// Writes the low halfword of engine A DISPCNT.
    pub fn set_dispcnt_a_lo(&mut self, halfword: u16) {
        self.engine_upper.set_dispcnt_lo(halfword);
    }

    /// Writes DISPCNT (display control) of engine A (0x04000000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_dispcnt_a(&mut self, word: u32) {
        self.engine_upper.set_dispcnt(word);
    }

    /// Writes the low halfword of engine B DISPCNT.
    pub fn set_dispcnt_b_lo(&mut self, halfword: u16) {
        self.engine_lower.set_dispcnt_lo(halfword);
    }

    /// Writes DISPCNT (display control) of engine B (0x04001000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_dispcnt_b(&mut self, word: u32) {
        self.engine_lower.set_dispcnt(word);
    }

    /// Set DISPSTAT7 register value
    pub fn set_dispstat7(&mut self, value: u16) {
        self.display_status_arm7.set(value);
    }

    /// Set DISPSTAT9 register value
    pub fn set_dispstat9(&mut self, value: u16) {
        self.display_status_arm9.set(value);
    }

    /// Writes BGxCNT (BG control) of engine A (0x04000000); the 2D engine uses it from the next
    /// scanline.
    pub fn set_bgcnt_a(&mut self, halfword: u16, index: usize) {
        self.engine_upper.set_bgcnt(halfword, index);
    }

    /// Writes BGxCNT (BG control) of engine B (0x04001000); the 2D engine uses it from the next
    /// scanline.
    pub fn set_bgcnt_b(&mut self, halfword: u16, index: usize) {
        self.engine_lower.set_bgcnt(halfword, index);
    }

    /// Writes BGxHOFS (BG X scroll) of engine A (0x04000000); the 2D engine uses it from the next
    /// scanline.
    pub fn set_bghofs_a(&mut self, halfword: u16, index: usize) {
        self.engine_upper.set_bghofs(halfword, index);
    }

    /// Writes BGxVOFS (BG Y scroll) of engine A (0x04000000); the 2D engine uses it from the next
    /// scanline.
    pub fn set_bgvofs_a(&mut self, halfword: u16, index: usize) {
        self.engine_upper.set_bgvofs(halfword, index);
    }

    /// Writes BGxHOFS (BG X scroll) of engine B (0x04001000); the 2D engine uses it from the next
    /// scanline.
    pub fn set_bghofs_b(&mut self, halfword: u16, index: usize) {
        self.engine_lower.set_bghofs(halfword, index);
    }

    /// Writes BGxVOFS (BG Y scroll) of engine B (0x04001000); the 2D engine uses it from the next
    /// scanline.
    pub fn set_bgvofs_b(&mut self, halfword: u16, index: usize) {
        self.engine_lower.set_bgvofs(halfword, index);
    }

    /// Writes BG2PA..PD (BG2 affine matrix) of engine A (0x04000000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg2p_a(&mut self, halfword: u16, index: usize) {
        self.engine_upper.set_bg2p(halfword, index, self.get_vcount());
    }

    /// Writes BG2PA..PD (BG2 affine matrix) of engine B (0x04001000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg2p_b(&mut self, halfword: u16, index: usize) {
        self.engine_lower.set_bg2p(halfword, index, self.get_vcount());
    }

    /// Writes BG3PA..PD (BG3 affine matrix) of engine A (0x04000000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg3p_a(&mut self, halfword: u16, index: usize) {
        self.engine_upper.set_bg3p(halfword, index, self.get_vcount());
    }

    /// Writes BG3PA..PD (BG3 affine matrix) of engine B (0x04001000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg3p_b(&mut self, halfword: u16, index: usize) {
        self.engine_lower.set_bg3p(halfword, index, self.get_vcount());
    }

    /// Writes BG2X (BG2 affine reference X) of engine A (0x04000000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg2x_a(&mut self, word: u32) {
        self.engine_upper.set_bg2x(word, self.get_vcount());
    }

    /// Writes BG2Y (BG2 affine reference Y) of engine A (0x04000000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg2y_a(&mut self, word: u32) {
        self.engine_upper.set_bg2y(word, self.get_vcount());
    }

    /// Writes BG3X (BG3 affine reference X) of engine A (0x04000000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg3x_a(&mut self, word: u32) {
        self.engine_upper.set_bg3x(word, self.get_vcount());
    }

    /// Writes BG3Y (BG3 affine reference Y) of engine A (0x04000000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg3y_a(&mut self, word: u32) {
        self.engine_upper.set_bg3y(word, self.get_vcount());
    }

    /// Writes BG2X (BG2 affine reference X) of engine B (0x04001000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg2x_b(&mut self, word: u32) {
        self.engine_lower.set_bg2x(word, self.get_vcount());
    }

    /// Writes BG2Y (BG2 affine reference Y) of engine B (0x04001000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg2y_b(&mut self, word: u32) {
        self.engine_lower.set_bg2y(word, self.get_vcount());
    }

    /// Writes BG3X (BG3 affine reference X) of engine B (0x04001000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg3x_b(&mut self, word: u32) {
        self.engine_lower.set_bg3x(word, self.get_vcount());
    }

    /// Writes BG3Y (BG3 affine reference Y) of engine B (0x04001000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bg3y_b(&mut self, word: u32) {
        self.engine_lower.set_bg3y(word, self.get_vcount());
    }

    /// Writes WIN0H (window 0 X range) of engine A (0x04000000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_win0h_a(&mut self, halfword: u16) {
        self.engine_upper.set_win0h(halfword);
    }

    /// Writes WIN1H (window 1 X range) of engine A (0x04000000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_win1h_a(&mut self, halfword: u16) {
        self.engine_upper.set_win1h(halfword);
    }

    /// Writes WIN0V (window 0 Y range) of engine A (0x04000000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_win0v_a(&mut self, halfword: u16) {
        self.engine_upper.set_win0v(halfword);
    }

    /// Writes WIN1V (window 1 Y range) of engine A (0x04000000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_win1v_a(&mut self, halfword: u16) {
        self.engine_upper.set_win1v(halfword);
    }

    /// Writes WIN0H (window 0 X range) of engine B (0x04001000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_win0h_b(&mut self, halfword: u16) {
        self.engine_lower.set_win0h(halfword);
    }

    /// Writes WIN1H (window 1 X range) of engine B (0x04001000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_win1h_b(&mut self, halfword: u16) {
        self.engine_lower.set_win1h(halfword);
    }

    /// Writes WIN0V (window 0 Y range) of engine B (0x04001000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_win0v_b(&mut self, halfword: u16) {
        self.engine_lower.set_win0v(halfword);
    }

    /// Writes WIN1V (window 1 Y range) of engine B (0x04001000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_win1v_b(&mut self, halfword: u16) {
        self.engine_lower.set_win1v(halfword);
    }

    /// Writes WININ (layers inside WIN0/WIN1) of engine A (0x04000000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_winin_a(&mut self, halfword: u16) {
        self.engine_upper.set_winin(halfword);
    }

    /// Writes WININ (layers inside WIN0/WIN1) of engine B (0x04001000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_winin_b(&mut self, halfword: u16) {
        self.engine_lower.set_winin(halfword);
    }

    /// Writes WINOUT (layers outside / in the OBJ window) of engine A (0x04000000); the 2D engine
    /// uses it from the next scanline.
    pub fn set_winout_a(&mut self, halfword: u16) {
        self.engine_upper.set_winout(halfword);
    }

    /// Writes WINOUT (layers outside / in the OBJ window) of engine B (0x04001000); the 2D engine
    /// uses it from the next scanline.
    pub fn set_winout_b(&mut self, halfword: u16) {
        self.engine_lower.set_winout(halfword);
    }

    /// Writes MOSAIC (mosaic size) of engine A (0x04000000); the 2D engine uses it from the next
    /// scanline.
    pub fn set_mosaic_a(&mut self, halfword: u16) {
        self.engine_upper.set_mosaic(halfword);
    }

    /// Writes MOSAIC (mosaic size) of engine B (0x04001000); the 2D engine uses it from the next
    /// scanline.
    pub fn set_mosaic_b(&mut self, halfword: u16) {
        self.engine_lower.set_mosaic(halfword);
    }

    /// Writes BLDCNT (color effect selection) of engine A (0x04000000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bldcnt_a(&mut self, halfword: u16) {
        self.engine_upper.set_bldcnt(halfword);
    }

    /// Writes BLDCNT (color effect selection) of engine B (0x04001000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bldcnt_b(&mut self, halfword: u16) {
        self.engine_lower.set_bldcnt(halfword);
    }

    /// Writes BLDALPHA (alpha blend weights) of engine A (0x04000000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bldalpha_a(&mut self, halfword: u16) {
        self.engine_upper.set_bldalpha(halfword);
    }

    /// Writes BLDALPHA (alpha blend weights) of engine B (0x04001000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bldalpha_b(&mut self, halfword: u16) {
        self.engine_lower.set_bldalpha(halfword);
    }

    /// Writes BLDY (brightness fade factor) of engine A (0x04000000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bldy_a(&mut self, byte: u8) {
        self.engine_upper.set_bldy(byte);
    }

    /// Writes BLDY (brightness fade factor) of engine B (0x04001000); the 2D engine uses it from
    /// the next scanline.
    pub fn set_bldy_b(&mut self, byte: u8) {
        self.engine_lower.set_bldy(byte);
    }

    /// Writes DISP3DCNT (3D display control); the 2D engine uses it from the next scanline.
    pub fn set_disp3dcnt(&mut self, halfword: u16) {
        self.engine_3d.set_disp3dcnt(halfword);
    }

    /// Writes MASTER_BRIGHT (screen fade) of engine A (0x04000000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_master_bright_a(&mut self, halfword: u16) {
        self.engine_upper.set_master_bright(halfword);
    }

    /// Writes MASTER_BRIGHT (screen fade) of engine B (0x04001000); the 2D engine uses it from the
    /// next scanline.
    pub fn set_master_bright_b(&mut self, halfword: u16) {
        self.engine_lower.set_master_bright(halfword);
    }

    /// Writes DISPCAPCNT (display capture control); the 2D engine uses it from the next scanline.
    pub fn set_dispcapcnt(&mut self, word: u32) {
        self.engine_upper.set_dispcapcnt(word);
    }

    /// Set VRAM bank configuration A
    pub fn set_vramcnt_a(&mut self, value: u8) {
        self.vram.set_cnt(0, value);
    }

    /// Set VRAM bank configuration B
    pub fn set_vramcnt_b(&mut self, value: u8) {
        self.vram.set_cnt(1, value);
    }

    /// Set VRAM bank configuration C
    pub fn set_vramcnt_c(&mut self, value: u8) {
        self.vram.set_cnt(2, value);
    }

    /// Set VRAM bank configuration D
    pub fn set_vramcnt_d(&mut self, value: u8) {
        self.vram.set_cnt(3, value);
    }

    /// Set VRAM bank configuration E
    pub fn set_vramcnt_e(&mut self, value: u8) {
        self.vram.set_cnt(4, value);
    }

    /// Set VRAM bank configuration F
    pub fn set_vramcnt_f(&mut self, value: u8) {
        self.vram.set_cnt(5, value);
    }

    /// Set VRAM bank configuration G
    pub fn set_vramcnt_g(&mut self, value: u8) {
        self.vram.set_cnt(6, value);
    }

    /// Set VRAM bank configuration H
    pub fn set_vramcnt_h(&mut self, value: u8) {
        self.vram.set_cnt(7, value);
    }

    /// Set VRAM bank configuration I
    pub fn set_vramcnt_i(&mut self, value: u8) {
        self.vram.set_cnt(8, value);
    }

    /// Set POWCNT1 register value
    pub fn set_powcnt1(&mut self, value: u16) {
        self.power_control_reg.set(value);
    }
}
