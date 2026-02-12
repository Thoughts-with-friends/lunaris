// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
use crate::gpu_root::Gpu;

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

    pub fn set_dispcnt_a_lo(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_dispcnt_a(&mut self, word: u32) {
        todo!()
    }

    pub fn set_dispcnt_b_lo(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_dispcnt_b(&mut self, word: u32) {
        todo!()
    }

    /// Set DISPSTAT7 register value
    pub fn set_dispstat7(&mut self, value: u16) {
        self.display_status_arm7.set(value);
    }

    /// Set DISPSTAT9 register value
    pub fn set_dispstat9(&mut self, value: u16) {
        self.display_status_arm9.set(value);
    }

    pub fn set_bgcnt_a(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bgcnt_b(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bghofs_a(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bgvofs_a(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bghofs_b(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bgvofs_b(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bg2p_a(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bg2p_b(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bg3p_a(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bg3p_b(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bg2x_a(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg2y_a(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg3x_a(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg3y_a(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg2x_b(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg2y_b(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg3x_b(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg3y_b(&mut self, word: u32) {
        todo!()
    }

    pub fn set_win0h_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win1h_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win0v_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win1v_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win0h_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win1h_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win0v_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win1v_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_winin_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_winin_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_winout_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_winout_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_mosaic_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_mosaic_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_bldcnt_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_bldcnt_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_bldalpha_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_bldalpha_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_bldy_a(&mut self, byte: u8) {
        todo!()
    }

    pub fn set_bldy_b(&mut self, byte: u8) {
        todo!()
    }

    pub fn set_disp3dcnt(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_master_bright_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_master_bright_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_dispcapcnt(&mut self, word: u32) {
        todo!()
    }

    /// Set VRAM bank configuration A
    pub fn set_vramcnt_a(&mut self, value: u8) {
        self.vramcnt_a.mst = (value & 0x7) as u32;
        self.vramcnt_a.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration B
    pub fn set_vramcnt_b(&mut self, value: u8) {
        self.vramcnt_b.mst = (value & 0x7) as u32;
        self.vramcnt_b.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration C
    pub fn set_vramcnt_c(&mut self, value: u8) {
        self.vramcnt_c.mst = (value & 0x7) as u32;
        self.vramcnt_c.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration D
    pub fn set_vramcnt_d(&mut self, value: u8) {
        self.vramcnt_d.mst = (value & 0x7) as u32;
        self.vramcnt_d.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration E
    pub fn set_vramcnt_e(&mut self, value: u8) {
        self.vramcnt_e.mst = (value & 0x7) as u32;
        self.vramcnt_e.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration F
    pub fn set_vramcnt_f(&mut self, value: u8) {
        self.vramcnt_f.mst = (value & 0x7) as u32;
        self.vramcnt_f.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration G
    pub fn set_vramcnt_g(&mut self, value: u8) {
        self.vramcnt_g.mst = (value & 0x7) as u32;
        self.vramcnt_g.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration H
    pub fn set_vramcnt_h(&mut self, value: u8) {
        self.vramcnt_h.mst = (value & 0x7) as u32;
        self.vramcnt_h.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration I
    pub fn set_vramcnt_i(&mut self, value: u8) {
        self.vramcnt_i.mst = (value & 0x7) as u32;
        self.vramcnt_i.enabled = (value & 0x80) != 0;
    }

    /// Set POWCNT1 register value
    pub fn set_powcnt1(&mut self, value: u16) {
        self.power_control_reg.set(value);
    }

    /// Write to GXFIFO command queue
    pub fn write_gxfifo(&mut self, _word: u32) {
        todo!()
    }

    pub fn write_fifo_direct(&mut self, address: u32, word: u32) {
        todo!()
    }

    pub fn set_clear_color(&mut self, word: u32) {
        todo!()
    }

    pub fn set_clear_depth(&mut self, word: u32) {
        todo!()
    }

    pub fn set_mtx_mode(&mut self, word: u32) {
        todo!()
    }
}
