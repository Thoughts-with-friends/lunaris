// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
use crate::gpu_root::Gpu;

impl Gpu {
    pub fn mtx_push(&mut self) {
        self.engine_3d.mtx_push();
    }

    pub fn mtx_pop(&mut self, word: u32) {
        self.engine_3d.mtx_pop(word);
    }

    pub fn mtx_identity(&mut self) {
        self.engine_3d.mtx_identity();
    }

    pub fn mtx_mult_4x4(&mut self, word: u32) {
        self.engine_3d.mtx_mult_4x4(word);
    }

    pub fn mtx_mult_4x3(&mut self, word: u32) {
        self.engine_3d.mtx_mult_4x3(word);
    }

    pub fn mtx_mult_3x3(&mut self, word: u32) {
        self.engine_3d.mtx_mult_3x3(word);
    }

    pub fn mtx_trans(&mut self, word: u32) {
        self.engine_3d.mtx_trans(word);
    }

    pub fn color(&mut self, word: u32) {
        self.engine_3d.color(word);
    }

    pub fn vtx_16(&mut self, word: u32) {
        // commented out in C++.
        let _ = word;
        // self.engine_3d.vtx_16(word);
    }

    pub fn set_polygon_attr(&mut self, word: u32) {
        self.engine_3d.set_polygon_attr(word);
    }

    pub fn set_teximage_param(&mut self, word: u32) {
        self.engine_3d.set_teximage_param(word);
    }

    pub fn set_toon_table(&mut self, address: u32, color: u16) {
        self.engine_3d.set_toon_table(address, color);
    }

    pub fn begin_vtxs(&mut self, word: u32) {
        self.engine_3d.begin_vtxs(word);
    }

    pub fn swap_buffers(&mut self, word: u32) {
        self.engine_3d.swap_buffers(word);
    }

    pub fn viewport(&mut self, word: u32) {
        self.engine_3d.viewport(word);
    }

    // Removed set_gxstat() after moving it to Emulator.
}
