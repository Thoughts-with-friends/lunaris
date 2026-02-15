// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp
//!
use lunaris_ds_mem_const::{PIXELS_PER_LINE, SCANLINES};
use std::collections::VecDeque;

use crate::gpu_3d::structs::{
    Disp3DCntReg, Gpu3D, GxStatReg, Matrix, Polygon, PolygonAttrReg, TexImageParamReg, Vertex,
    ViewportReg,
};

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
            gxfifo: VecDeque::new(),
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

    // ============= private method =============

    /// Perspective-Correct Linear Interpolation
    fn interpolate(
        &self,
        step: u64,
        step_count: u64,
        value_start: i64,
        value_end: i64,
        depth_scale_start: i32,
        depth_scale_end: i32,
    ) -> i64 {
        let pixel = step as i64;
        let pixel_range = step_count as i64;
        let w1 = depth_scale_start as i64;
        let w2 = depth_scale_end as i64;

        let mut bark = (pixel_range - pixel) * (value_start * w2);
        bark += pixel * (value_end * w1);

        let mut denom = (pixel_range - pixel) * w2;
        denom += pixel * w1;

        bark / denom
    }

    // Moved to impl Matrix
    // fn get_identity_matrix(&mut self, mtx: &mut Matrix);
    // pub fn read_command(&mut self) -> Option<GxCommand>;
    // fn write_command(&mut self, cmd: GxCommand);

    // fn exec_command(&mut self) {
    //     todo!()
    // }

    fn add_mult_param(&mut self) {
        todo!()
    }

    fn matrix_mult(&mut self) {
        todo!()
    }

    fn update_clip_matrix(&mut self) {
        todo!()
    }

    fn clip(
        &mut self,
        v_list: &mut [Vertex],
        mut v_len: usize,
        clip_start: usize,
        add_attributes: bool,
    ) -> usize {
        v_len = self.clip_plane(0, v_list, v_len, clip_start, add_attributes);
        v_len = self.clip_plane(1, v_list, v_len, clip_start, add_attributes);
        v_len = self.clip_plane(2, v_list, v_len, clip_start, add_attributes);
        v_len
    }

    pub fn clip_plane(
        &mut self,
        plane: usize,
        v_list: &mut [Vertex],
        mut v_len: usize,
        clip_start: usize,
        add_attributes: bool,
    ) -> usize {
        let mut temp_v_list: Vec<Vertex> = vec![Vertex::default(); 10];
        let mut clip_index = clip_start;

        if clip_start == 2 {
            temp_v_list[0] = v_list[0].clone();
            temp_v_list[1] = v_list[1].clone();
        }

        // Clip everything higher than w
        for i in clip_start..v_len {
            let prev_v = if i == 0 { v_len - 1 } else { i - 1 };
            let next_v = if i + 1 >= v_len { 0 } else { i + 1 };

            let v = v_list[i].clone();

            if v.coords[plane] > v.coords[3] {
                if plane == 2 && !self.current_poly_attr.render_far_intersect {
                    return 0;
                }

                let vp = v_list[prev_v].clone();
                if vp.coords[plane] <= vp.coords[3] {
                    self.clip_vertex(
                        plane,
                        &mut temp_v_list[clip_index],
                        &v,
                        &vp,
                        1,
                        add_attributes,
                    );
                    clip_index += 1;
                }

                let vn = v_list[next_v].clone();
                if vn.coords[plane] <= vn.coords[3] {
                    self.clip_vertex(
                        plane,
                        &mut temp_v_list[clip_index],
                        &v,
                        &vn,
                        1,
                        add_attributes,
                    );
                    clip_index += 1;
                }
            } else {
                temp_v_list[clip_index] = v;
                clip_index += 1;
            }
        }

        v_len = clip_index;
        clip_index = clip_start;

        // Clip everything lower than -w
        for i in clip_start..v_len {
            let prev_v = if i == 0 { v_len - 1 } else { i - 1 };
            let next_v = if i + 1 >= v_len { 0 } else { i + 1 };

            let v = temp_v_list[i].clone();

            if v.coords[plane] < -v.coords[3] {
                let vp = temp_v_list[prev_v].clone();
                if vp.coords[plane] >= -vp.coords[3] {
                    self.clip_vertex(plane, &mut v_list[clip_index], &v, &vp, -1, add_attributes);
                    clip_index += 1;
                }

                let vn = temp_v_list[next_v].clone();
                if vn.coords[plane] >= -vn.coords[3] {
                    self.clip_vertex(plane, &mut v_list[clip_index], &v, &vn, -1, add_attributes);
                    clip_index += 1;
                }
            } else {
                v_list[clip_index] = v;
                clip_index += 1;
            }
        }

        clip_index
    }

    pub fn clip_vertex(
        &mut self,
        plane: usize,
        v_list: &mut Vertex, // output
        v_out: &Vertex,      // outside vertex
        v_in: &Vertex,       // inside vertex
        side: i32,
        add_attributes: bool,
    ) {
        // Copied logic from melonDS version
        let factor_num: i64 = v_in.coords[3] as i64 - (side as i64 * v_in.coords[plane] as i64);

        let factor_den: i32 = (factor_num
            - (v_out.coords[3] as i64 - (side as i64 * v_out.coords[plane] as i64)))
            as i32;

        if factor_den == 0 {
            panic!("Error: factor_den equals zero!");
        }

        // Helper closure for interpolation
        let interp = |vin: i32, vout: i32| -> i32 {
            vin + (((vout - vin) as i64 * factor_num) / factor_den as i64) as i32
        };

        if plane != 0 {
            v_list.coords[0] = interp(v_in.coords[0], v_out.coords[0]);
        }
        if plane != 1 {
            v_list.coords[1] = interp(v_in.coords[1], v_out.coords[1]);
        }
        if plane != 2 {
            v_list.coords[2] = interp(v_in.coords[2], v_out.coords[2]);
        }

        v_list.coords[3] = interp(v_in.coords[3], v_out.coords[3]);
        v_list.coords[plane] = side * v_list.coords[3];

        if add_attributes {
            v_list.colors[0] = interp(v_in.colors[0], v_out.colors[0]);
            v_list.colors[1] = interp(v_in.colors[1], v_out.colors[1]);
            v_list.colors[2] = interp(v_in.colors[2], v_out.colors[2]);

            v_list.texcoords[0] = interp(v_in.texcoords[0], v_out.texcoords[0]);
            v_list.texcoords[1] = interp(v_in.texcoords[1], v_out.texcoords[1]);
        }

        v_list.clipped = true;
    }

    pub fn add_vertex(&mut self) {
        // if (geo_vert_count >= 6188)
        //     return;
        // int64_t coords[4];
        // coords[0] = (int64_t)(int16_t)current_vertex[0];
        // coords[1] = (int64_t)(int16_t)current_vertex[1];
        // coords[2] = (int64_t)(int16_t)current_vertex[2];
        // coords[3] = 0x1000;

        // update_clip_mtx();

        // Vertex* vtx = &vertex_list[vertex_list_count];

        // vtx->coords[0] = (coords[0]*clip_mtx.m[0][0] + coords[1]*clip_mtx.m[1][0] +
        //         coords[2]*clip_mtx.m[2][0] + coords[3]*clip_mtx.m[3][0]) >> 12;
        // vtx->coords[1] = (coords[0]*clip_mtx.m[0][1] + coords[1]*clip_mtx.m[1][1] +
        //         coords[2]*clip_mtx.m[2][1] + coords[3]*clip_mtx.m[3][1]) >> 12;
        // vtx->coords[2] = (coords[0]*clip_mtx.m[0][2] + coords[1]*clip_mtx.m[1][2] +
        //         coords[2]*clip_mtx.m[2][2] + coords[3]*clip_mtx.m[3][2]) >> 12;
        // vtx->coords[3] = (coords[0]*clip_mtx.m[0][3] + coords[1]*clip_mtx.m[1][3] +
        //         coords[2]*clip_mtx.m[2][3] + coords[3]*clip_mtx.m[3][3]) >> 12;

        // if (TEXIMAGE_PARAM.transformation_mode == 3)
        // {
        //     int16_t texcoords[2];
        //     texcoords[0] = current_texcoords[0];
        //     texcoords[1] = current_texcoords[1];
        //     current_texcoords[0] = ((coords[0] * texture_mtx.m[0][0] + coords[1] * texture_mtx.m[1][0]
        //             + coords[2] * texture_mtx.m[2][0]) >> 24) + texcoords[0];
        //     current_texcoords[1] = ((coords[0] * texture_mtx.m[0][1] + coords[1] * texture_mtx.m[1][1]
        //             + coords[2] * texture_mtx.m[2][1]) >> 24) + texcoords[1];
        // }

        // vtx->colors[0] = ((current_color & 0x1F) << 12) + 0xFFF;
        // vtx->colors[1] = (((current_color >> 5) & 0x1F) << 12) + 0xFFF;
        // vtx->colors[2] = (((current_color >> 10) & 0x1F) << 12) + 0xFFF;
        // vtx->texcoords[0] = (int16_t)current_texcoords[0];
        // vtx->texcoords[1] = (int16_t)current_texcoords[1];
        // vtx->clipped = false;

        // vertex_list_count++;
        // switch (POLYGON_TYPE)
        // {
        //     case 0:
        //         if (vertex_list_count == 3)
        //         {
        //             add_polygon();
        //             consecutive_polygons++;
        //             vertex_list_count = 0;
        //         }
        //         break;
        //     case 1:
        //         if (vertex_list_count == 4)
        //         {
        //             add_polygon();
        //             consecutive_polygons++;
        //             vertex_list_count = 0;
        //         }
        //         break;
        //     case 2:
        //         if (consecutive_polygons & 0x1)
        //         {
        //             swap(vertex_list[0], vertex_list[1]);

        //             add_polygon();
        //             consecutive_polygons++;
        //             vertex_list_count = 2;
        //             vertex_list[1] = vertex_list[2];
        //         }
        //         else if (vertex_list_count == 3)
        //         {
        //             add_polygon();
        //             consecutive_polygons++;
        //             vertex_list_count = 2;
        //             vertex_list[0] = vertex_list[1];
        //             vertex_list[1] = vertex_list[2];
        //         }
        //         break;
        //     case 3:
        //         if (vertex_list_count == 4)
        //         {
        //             swap(vertex_list[2], vertex_list[3]);
        //             add_polygon();
        //             consecutive_polygons++;
        //             vertex_list_count = 2;

        //             vertex_list[0] = vertex_list[3];
        //             vertex_list[1] = vertex_list[2];
        //         }
        //         break;
        //     default:
        //         printf("\nUnrecognized POLYGON_TYPE %d", POLYGON_TYPE);
        //         exit(1);
        // }
    }

    fn add_polygon(&mut self) {
        todo!()
    }

    fn request_fifo_dma(&mut self) {
        todo!()
    }

    // ============= private method =============

    pub fn power_on(&mut self) {
        // Clear GXPIPE and GXFIFO
        self.gxpipe.clear();
        self.gxfifo.clear();

        self.set_disp3dcnt(0);
        self.mult_params.set_identity();
        self.projection_mtx.set_identity();
        self.vector_mtx.set_identity();
        self.modelview_mtx.set_identity();
        self.texture_mtx.set_identity();

        self.mult_params_index = 0;
        self.geo_vert_count = 0;
        self.geo_poly_count = 0;
        self.rend_vert_count = 0;
        self.rend_poly_count = 0;
        self.vtx_16_index = 0;
        self.model_view_sp = 0;
        self.vertex_list_count = 0;
        self.clip_dirty = true;

        self.cycles = 0;
        self.param_count = 0;
        self.cmd_param_count = 0;
        self.cmd_count = 0;
        self.swap_buffers = false;
        self.gxstat.boxtest_result = false;
        self.gxstat.box_pos_vec_busy = false;
        self.gxstat.mtx_stack_busy = false;
        self.gxstat.mtx_overflow = false;
        self.gxstat.geo_busy = false;
        self.gxstat.gxfifo_irq_stat = 0;
        self.teximage_param.format = 0;

        self.current_color = 0x7FFF;
        self.clear_depth = 0x7FFF;

        self.last_poly_strip = None;
    }

    // Moved to struct Emulator method (Because use emulator method)
    // pub fn run_3d(&mut self, cycles: u64)

    pub fn render_scanline(
        &mut self,
        framebuffer: &mut [u32],
        bg_priorities: &[u8],
        bg0_priority: u8,
    ) {
        todo!()
    }

    // Moved to struct Emulator method (Because use emulator method)
    // pub fn run(&mut self, cycles_to_run: u64)

    pub fn end_of_frame(&mut self) {
        todo!()
    }

    pub fn check_fifo_dma(&mut self) {
        todo!()
    }

    // Moved to struct Emulator method (Because use emulator method)
    // pub fn check_fifo_irq(&mut self);

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

    // moved execute.rs: pub fn normal(&mut self)

    pub fn set_polygon_attr(&mut self, word: u32) {
        self.polygon_attr.set(word);
    }

    pub fn set_teximage_param(&mut self, word: u32) {
        self.teximage_param.set(word);
    }

    /// # Panics
    /// address : 0..32
    #[expect(unused)]
    pub fn set_toon_table(&mut self, address: u32, color: u16) {
        self.toon_table[address as usize] = color;
    }

    pub fn begin_vtxs(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("BEGIN_VTXS: {word:08X}");

        self.polygon_type = word & 0x3;
    }

    pub fn swap_buffers(&mut self, word: u32) {
        self.swap_buffers = true;
        self.flush_mode = (word & 0x3) as i32;
    }

    pub fn viewport(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("VIEWPORT: {word:08X}");

        //viewport y-coords are upside down
        self.viewport.x1 = (word & 0xFF) as u8;
        self.viewport.y1 = ((191 - ((word >> 8) & 0xFF)) & 0xFF) as u8;
        self.viewport.x2 = ((word >> 16) & 0xFF) as u8;
        self.viewport.y2 = ((191 - ((word >> 24) & 0xFF)) & 0xFF) as u8;
    }

    pub fn box_test(&mut self) {
        #[cfg(feature = "tracing")]
        tracing::debug!("BOX_TEST");
        self.gxstat.boxtest_result = true;

        let mut cube: [Vertex; 8] = core::array::from_fn(|_| Vertex::default());
        let mut face: [Vertex; 10] = core::array::from_fn(|_| Vertex::default());
        let mut coords0 = [0_i16; 3];
        let mut coords1 = [0_i16; 3];

        coords0[0] = (self.cmd_params[0] & 0xFFFF) as i16;
        coords0[1] = (self.cmd_params[0] >> 16) as i16;
        coords0[2] = (self.cmd_params[1] & 0xFFFF) as i16;
        coords1[0] = (self.cmd_params[1] >> 16) as i16;
        coords1[1] = (self.cmd_params[2] & 0xFFFF) as i16;
        coords1[2] = (self.cmd_params[2] >> 16) as i16;

        cube[0].coords[0] = coords0[0] as i32;
        cube[0].coords[1] = coords0[1] as i32;
        cube[0].coords[2] = coords0[2] as i32;
        cube[1].coords[0] = coords1[0] as i32;
        cube[1].coords[1] = coords0[1] as i32;
        cube[1].coords[2] = coords0[2] as i32;
        cube[2].coords[0] = coords1[0] as i32;
        cube[2].coords[1] = coords1[1] as i32;
        cube[2].coords[2] = coords0[2] as i32;
        cube[3].coords[0] = coords0[0] as i32;
        cube[3].coords[1] = coords1[1] as i32;
        cube[3].coords[2] = coords0[2] as i32;
        cube[4].coords[0] = coords0[0] as i32;
        cube[4].coords[1] = coords1[1] as i32;
        cube[4].coords[2] = coords1[2] as i32;
        cube[5].coords[0] = coords0[0] as i32;
        cube[5].coords[1] = coords0[1] as i32;
        cube[5].coords[2] = coords1[2] as i32;
        cube[6].coords[0] = coords1[0] as i32;
        cube[6].coords[1] = coords0[1] as i32;
        cube[6].coords[2] = coords1[2] as i32;
        cube[7].coords[0] = coords1[0] as i32;
        cube[7].coords[1] = coords1[1] as i32;
        cube[7].coords[2] = coords1[2] as i32;

        self.update_clip_matrix();

        for c in &mut cube {
            let x = c.coords[0];
            let y = c.coords[1];
            let z = c.coords[2];

            c.coords[0] = (x * self.clip_mtx.m[0][0]
                + y * self.clip_mtx.m[1][0]
                + z * self.clip_mtx.m[2][0]
                + 0x1000 * self.clip_mtx.m[3][0])
                >> 12;
            c.coords[1] = (x * self.clip_mtx.m[0][1]
                + y * self.clip_mtx.m[1][1]
                + z * self.clip_mtx.m[2][1]
                + 0x1000 * self.clip_mtx.m[3][1])
                >> 12;
            c.coords[2] = (x * self.clip_mtx.m[0][2]
                + y * self.clip_mtx.m[1][2]
                + z * self.clip_mtx.m[2][2]
                + 0x1000 * self.clip_mtx.m[3][2])
                >> 12;
            c.coords[3] = (x * self.clip_mtx.m[0][3]
                + y * self.clip_mtx.m[1][3]
                + z * self.clip_mtx.m[2][3]
                + 0x1000 * self.clip_mtx.m[3][3])
                >> 12;
        }

        let mut vertices;

        //Front face
        face[0] = cube[0].clone();
        face[1] = cube[1].clone();
        face[2] = cube[2].clone();
        face[3] = cube[3].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
            return;
        }

        //Back face
        face[0] = cube[4].clone();
        face[1] = cube[5].clone();
        face[2] = cube[6].clone();
        face[3] = cube[7].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
            return;
        }

        //Left face
        face[0] = cube[0].clone();
        face[1] = cube[3].clone();
        face[2] = cube[4].clone();
        face[3] = cube[5].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
            return;
        }

        //Right face
        face[0] = cube[1].clone();
        face[1] = cube[2].clone();
        face[2] = cube[7].clone();
        face[3] = cube[6].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
            return;
        }

        //Bottom face
        face[0] = cube[0].clone();
        face[1] = cube[1].clone();
        face[2] = cube[6].clone();
        face[3] = cube[5].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
            return;
        }

        //Top face
        face[0] = cube[2].clone();
        face[1] = cube[3].clone();
        face[2] = cube[4].clone();
        face[3] = cube[7].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
        }
    }

    pub fn vec_test(&mut self) {
        todo!()
    }

    // Moved to struct Emulator method (Because use emulator method)
    // pub fn set_gxstat(&mut self, word: u32)
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

// to run exec_command
impl Gpu3D {
    pub fn mtx_mult(&mut self, update_vector: bool) {
        let mut temp = Matrix::zeros();
        let mut target = &mut Matrix::zeros();

        if self.mtx_mode != 3 {
            self.clip_dirty = true;
        }

        match self.mtx_mode {
            0 => target = &mut self.projection_mtx,
            1 => target = &mut self.modelview_mtx,
            2 => {
                target = &mut self.modelview_mtx;

                // Only multiply vector if command is not MTX_SCALE
                if update_vector {
                    temp.set(&self.vector_mtx);

                    // vec-matrix setting
                    for i in 0..4 {
                        for j in 0..4 {
                            let mut temp_calc = 0;
                            for k in 0..4 {
                                temp_calc += (self.mult_params.m[i][k] * temp.m[k][j]) as i64;
                            }
                            self.vector_mtx.m[i][j] = (temp_calc >> 12) as i32;
                        }
                    }
                }
            }
            3 => self.texture_mtx.set(&self.texture_stack),
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Unrecognized MTX_MODE {} for MTX_POP", self.mtx_mode);
            }
        }

        temp.set(target);

        // target setting
        for i in 0..4 {
            for j in 0..4 {
                let mut temp_calc = 0;
                for k in 0..4 {
                    temp_calc += (self.mult_params.m[i][k] * temp.m[k][j]) as i64;
                }
                target.m[i][j] = (temp_calc >> 12) as i32;
            }
        }

        // Reset the mult matrix for further use
        self.mult_params.set_identity();
        self.mult_params_index = 0;
    }

    pub fn normal(&mut self) {
        // Let's calculate some lighting!
        self.normal_vector[0] = (((self.cmd_params[0] & 0x3FF) << 6) >> 6) as i16;
        self.normal_vector[1] = ((((self.cmd_params[0] >> 10) & 0x3FF) << 6) >> 6) as i16;
        self.normal_vector[2] = ((((self.cmd_params[0] >> 20) & 0x3FF) << 6) >> 6) as i16;
        let normal_vector_i32 = self.normal_vector.map(|x| x as i32);

        if self.teximage_param.transformation_mode == 2 {
            self.current_texcoords[0] += ((normal_vector_i32[0] * self.texture_mtx.m[0][0]
                + normal_vector_i32[1] * self.texture_mtx.m[1][0]
                + normal_vector_i32[2] * self.texture_mtx.m[2][0])
                >> 21) as i16;
            self.current_texcoords[1] += ((normal_vector_i32[0] * self.texture_mtx.m[0][1]
                + normal_vector_i32[1] * self.texture_mtx.m[1][1]
                + normal_vector_i32[2] * self.texture_mtx.m[2][1])
                >> 21) as i16;
        }

        let mut normal_vec = [0, 0, 0];
        for (i, normal) in normal_vec.iter_mut().enumerate() {
            *normal = normal_vector_i32[0] * self.vector_mtx.m[0][i]
                + normal_vector_i32[1] * self.vector_mtx.m[1][i]
                + normal_vector_i32[2] * self.vector_mtx.m[2][i];
            *normal >>= 12;
        }

        let mut r = (self.emission_color & 0x1F) as u32;
        let mut g = ((self.emission_color >> 5) & 0x1F) as u32;
        let mut b = ((self.emission_color >> 10) & 0x1F) as u32;

        for light in 0..4 {
            // if !(self.polygon_attr.light_enable >= 0 & (1 << light)) {
            //     continue;
            // }

            let lr = (self.light_color[light] & 0x1F) as u32;
            let lg = ((self.light_color[light] >> 5) & 0x1F) as u32;
            let lb = ((self.light_color[light] >> 10) & 0x1F) as u32;

            let light_direction_i32 = self.light_direction.map(|vec| vec.map(|x| x as i32));

            let mut diffuse_level = (-(light_direction_i32[light][0] * normal_vec[0]
                + light_direction_i32[light][1] * normal_vec[1]
                + light_direction_i32[light][2] * normal_vec[2]))
                >> 10;

            // Overflow handling taken from melonDS (same goes for specular)
            diffuse_level = diffuse_level.clamp(0, 0xFF);

            let mut shine_level = -(((light_direction_i32[light][0] >> 1) * normal_vec[0]
                + (light_direction_i32[light][1] >> 1) * normal_vec[1]
                + ((light_direction_i32[light][2] - 0x200) >> 1) * normal_vec[2])
                >> 10);

            if shine_level < 0 {
                shine_level = 0;
            } else if shine_level > 0xFF {
                shine_level = (0x100 - shine_level) & 0xFF;
            }

            // 2*shine_level^2 - 1.0
            shine_level = ((shine_level * shine_level) >> 7) - 0x100;

            if shine_level < 0 {
                shine_level = 0;
            }

            if self.using_shine_table {
                shine_level >>= 1;
                shine_level = self.shine_table[shine_level as usize] as i32;
            }

            r += ((lr as i32 * (self.specular_color & 0x1F) as i32 * shine_level) >> 13) as u32;
            g += ((lg as i32 * ((self.specular_color >> 5) & 0x1F) as i32 * shine_level) >> 13)
                as u32;
            b += ((lb as i32 * ((self.specular_color >> 10) & 0x1F) as i32 * shine_level) >> 13)
                as u32;

            r += ((lr as i32 * (self.diffuse_color & 0x1F) as i32 * diffuse_level) >> 13) as u32;
            g += ((lg as i32 * ((self.diffuse_color >> 5) & 0x1F) as i32 * diffuse_level) >> 13)
                as u32;
            b += ((lb as i32 * ((self.diffuse_color >> 10) & 0x1F) as i32 * diffuse_level) >> 13)
                as u32;

            r += (lr * (self.ambient_color & 0x1F) as u32) >> 5;
            g += (lg * ((self.ambient_color >> 5) & 0x1F) as u32) >> 5;
            b += (lb * ((self.ambient_color >> 10) & 0x1F) as u32) >> 5;

            if r > 0x1F {
                r = 0x1F;
            }
            if g > 0x1F {
                g = 0x1F;
            }
            if b > 0x1F {
                b = 0x1F;
            }
        }

        self.current_color = r + (g << 5) + (b << 10);
    }
}
