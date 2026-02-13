// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp
//!
use std::collections::VecDeque;

use lunaris_ds_mem_const::{PIXELS_PER_LINE, SCANLINES};

use crate::gpu_3d::structs::*;

impl Gpu3D {
    pub fn new() -> Self {
        Self {
            cycles: 100,
            disp3dcnt: Disp3DCntReg::default(),
            polygon_attr: PolygonAttrReg::default(),
            teximage_param: TexImageParamReg::default(),
            toon_table: [0; 32],
            pltt_base: 0,
            viewport: ViewportReg::default(),
            gxstat: GxStatReg::default(),
            polygon_type: 0,
            clear_depth: 0,
            clear_color: 0,
            flush_mode: 0,
            gfxifo: VecDeque::new(),
            gxpipe: VecDeque::new(),
            cmd_params: vec![0; 32],
            param_count: 0,
            cmd_param_count: 0,
            cmd_count: 0,
            total_params: 0,
            current_cmd: 0,
            current_poly_attr: PolygonAttrReg::default(),
            current_color: 0,
            current_vertex: [0, 0, 0],
            current_texcoords: [0, 0],
            z_buffer: vec![vec![0; PIXELS_PER_LINE]; SCANLINES],
            trans_poly_ids: vec![0; PIXELS_PER_LINE],
            swap_buffers: false,
            geo_vert: vec![Vertex::default(); 6188],
            rend_vert: vec![Vertex::default(); 6188],
            geo_poly: vec![Polygon::default(); 2048],
            rend_poly: vec![Polygon::default(); 2048],
            last_poly_strip: None,
            vertex_list: vec![Vertex::default(); 10],
            vertex_list_count: 0,
            geo_vert_count: 0,
            rend_vert_count: 0,
            geo_poly_count: 0,
            rend_poly_count: 0,
            consecutive_polygons: 0,
            vtx_16_index: 0,
            mtx_mode: 0,
            projection_mtx: Matrix::default(),
            vector_mtx: Matrix::default(),
            modelview_mtx: Matrix::default(),
            texture_mtx: Matrix::default(),
            projection_stack: Matrix::default(),
            texture_stack: Matrix::default(),
            modelview_stack: vec![Matrix::default(); 0x20],
            vector_stack: vec![Matrix::default(); 0x20],
            clip_mtx: Matrix::default(),
            clip_dirty: false,
            model_view_sp: 0,
            emission_color: 0,
            ambient_color: 0,
            diffuse_color: 0,
            specular_color: 0,
            light_color: [0; 4],
            light_direction: [[0; 3]; 4],
            normal_vector: [0; 3],
            shine_table: vec![0; 128],
            using_shine_table: false,
            vec_test_result: [0; 3],
            mult_params: Matrix::default(),
            mult_params_index: 0,
        }
    }

    pub fn power_on(&mut self) {
        todo!()
    }

    pub fn exec_command(&mut self) {
        todo!()
    }

    pub fn render_scanline(
        &mut self,
        framebuffer: &mut [u32],
        bg_priorities: &[u8],
        bg0_priority: u8,
    ) {
        todo!()
    }

    pub fn run(&mut self, cycles_to_run: u64) {
        if self.swap_buffers {
            return;
        }
        if self.cycles == 0 && self.gxpipe.is_empty() {
            self.cycles = 0;
            return;
        }

        self.cycles -= cycles_to_run;
        while self.cycles == 0 && !self.gxpipe.is_empty() {
            self.exec_command();
        }
    }

    pub fn end_of_frame(&mut self) {
        todo!()
    }

    pub fn check_fifo_dma(&mut self) {
        todo!()
    }

    pub fn check_fifo_irq(&mut self) {
        todo!()
    }

    pub fn write_gxfifo(&mut self, word: u32) {
        todo!()
    }

    pub fn write_fifo_direct(&mut self, address: u32, word: u32) {
        todo!()
    }

    pub fn get_disp3dcnt(&self) -> u16 {
        todo!()
    }

    pub fn get_gxstat(&self) -> u32 {
        todo!()
    }

    pub fn get_vert_count(&self) -> u16 {
        todo!()
    }

    pub fn get_poly_count(&self) -> u16 {
        todo!()
    }

    pub fn read_clip_mtx(&self, address: u32) -> u32 {
        todo!()
    }

    pub fn read_vec_mtx(&self, address: u32) -> u32 {
        todo!()
    }

    pub fn read_vec_test(&self, address: u32) -> u16 {
        todo!()
    }

    pub fn set_disp3dcnt(&mut self, halfword: u16) {
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

    pub fn mtx_push(&mut self) {
        todo!()
    }

    pub fn mtx_pop(&mut self, word: u32) {
        todo!()
    }

    pub fn mtx_identity(&mut self) {
        todo!()
    }

    pub fn mtx_mult_4x4(&mut self, word: u32) {
        todo!()
    }

    pub fn mtx_mult_4x3(&mut self, word: u32) {
        todo!()
    }

    pub fn mtx_mult_3x3(&mut self, word: u32) {
        todo!()
    }

    pub fn mtx_trans(&mut self, word: u32) {
        todo!()
    }

    pub fn color(&mut self, word: u32) {
        todo!()
    }

    pub fn normal(&mut self) {
        todo!()
    }

    pub fn set_polygon_attr(&mut self, word: u32) {
        todo!()
    }

    pub fn set_teximage_param(&mut self, word: u32) {
        todo!()
    }

    pub fn set_toon_table(&mut self, address: u32, color: u16) {
        todo!()
    }

    pub fn begin_vtxs(&mut self, word: u32) {
        todo!()
    }

    pub fn swap_buffers(&mut self, word: u32) {
        todo!()
    }

    pub fn viewport(&mut self, word: u32) {
        todo!()
    }

    pub fn box_test(&mut self) {
        todo!()
    }

    pub fn vec_test(&mut self) {
        todo!()
    }

    pub fn set_gxstat(&mut self, word: u32) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_gpu3d() {
        // Initialize the Gpu3D struct with basic values
        let gpu = Gpu3D::new();

        // Example: test if initialization works without overflow
        assert_eq!(gpu.cycles, 100);
        assert_eq!(gpu.geo_vert.len(), 6188);
        assert_eq!(gpu.geo_poly.len(), 2048);
    }
}
