// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp
//!
use crate::gpu_3d::structs::{Gpu3D, Matrix, Polygon, Vertex};

fn safe_clone<T>(dest: &mut Vec<T>, src: &mut Vec<T>, size: usize)
where
    T: Default + Clone,
{
    let src: Vec<T> = core::mem::take(src);
    for (d, s) in dest.iter_mut().zip(src.iter()).take(size) {
        *d = s.clone();
    }
    let _ = core::mem::replace(dest, src);
}

impl Gpu3D {
    // ============= private method =============
    // moved geometry.rs
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
    // pub fn render_scanline;

    // Moved to struct Emulator method (Because use emulator method)
    // pub fn run(&mut self, cycles_to_run: u64)
    // pub fn check_fifo_dma(&mut self);

    pub fn end_of_frame(&mut self) {
        // #[cfg(feature = "tracing")]
        // tracing::info!("SWAP_BUFFERS");

        if self.swap_buffers {
            let geo_vert_count = self.geo_vert_count as usize;
            let geo_poly_count = self.geo_poly_count as usize;

            safe_clone(&mut self.rend_vert, &mut self.geo_vert, geo_vert_count);
            safe_clone(&mut self.rend_poly, &mut self.geo_poly, geo_poly_count);

            // #[cfg(feature = "tracing")]
            // tracing::info!("Geo_vert_count: {}", geo_vert_count);
            // tracing::info!("Geo_poly_count: {}", geo_poly_count);

            self.rend_vert_count = geo_vert_count as i32;
            self.rend_poly_count = geo_poly_count as i32;
            self.geo_vert_count = 0;
            self.geo_poly_count = 0;

            for i in 0..self.rend_poly_count as usize {
                let vert_index = self.rend_poly[i].vert_index as usize;
                self.rend_poly[i].top_y = 256;
                self.rend_poly[i].bottom_y = 0;

                for j in 0..self.rend_poly[i].vertices as usize {
                    #[cfg(feature = "tracing")]
                    tracing::info!(
                        "Color: (R, G, B) = ({}, {}, {})",
                        self.rend_vert[vert_index + j].colors[0] & 0xFFFF,
                        self.rend_vert[vert_index + j].colors[1] & 0xFFFF,
                        self.rend_vert[vert_index + j].colors[2] & 0xFFFF
                    );

                    let xx = self.rend_vert[vert_index + j].coords[0];
                    let yy = self.rend_vert[vert_index + j].coords[1];
                    let ww = self.rend_vert[vert_index + j].coords[3];

                    #[cfg(feature = "tracing")]
                    tracing::info!("Coords: (x, y, w) = ({}, {}, {})", xx, yy, ww);

                    let final_x: i32;
                    let final_y: i32;

                    if ww == 0 {
                        #[cfg(feature = "tracing")]
                        tracing::info!("Poly {} ww == 0?", i);
                    } else {
                        let width = self.viewport.x2 - self.viewport.x1 + 1;
                        let height = self.viewport.y1 - self.viewport.y2 + 1;
                        // let width = (self.viewport.x2 as u16 - self.viewport.x1 as u16 + 1) & 0x1FF;
                        // let height = (self.viewport.y1 - self.viewport.y2 + 1) & 0xFF;

                        // NOTE: (x2 - x1, y2 - y1)?
                        // let width = (self.viewport.x2 - self.viewport.x1 + 1) & 0x1FF;
                        // let height = self.viewport.y2 - self.viewport.y1 + 1;

                        let screen_x =
                            (((xx + ww) * width as i32) / (ww << 1)) + self.viewport.x1 as i32;
                        let screen_y =
                            (((-yy + ww) * height as i32) / (ww << 1)) + self.viewport.y2 as i32;

                        final_x = screen_x & 0x1FF;
                        final_y = screen_y & 0xFF;

                        #[cfg(feature = "tracing")]
                        tracing::info!("Screen shit: ({}, {})", final_x, final_y);

                        if final_y < self.rend_poly[i].top_y as i32 {
                            self.rend_poly[i].top_y = final_y as u16;
                        }
                        if final_y > self.rend_poly[i].bottom_y as i32 {
                            self.rend_poly[i].bottom_y = final_y as u16;
                        }

                        self.rend_vert[vert_index + j].coords[0] = final_x;
                        self.rend_vert[vert_index + j].coords[1] = final_y;
                        if (self.flush_mode & 0x2) != 0 {
                            self.rend_vert[vert_index + j].coords[2] = ww;
                        }
                    }
                }
            }

            //Sort polygons by translucency
            let mut index = 0;
            let mut opaque_count = 0;
            let mut temp: Vec<Polygon> = Vec::new(); // [2048]

            for i in 0..self.rend_poly_count as usize {
                if !self.rend_poly[i].translucent {
                    temp[index] = self.rend_poly[i].clone();
                    index += 1;
                    opaque_count += 1;
                }
            }

            for i in 0..self.rend_poly_count as usize {
                if self.rend_poly[i].translucent {
                    temp[index] = self.rend_poly[i].clone();
                    index += 1;
                }
            }

            safe_clone(
                &mut self.rend_poly,
                &mut temp,
                self.rend_poly_count as usize,
            );

            // y-sorting: opaque = false -> translucent = true
            if (self.flush_mode & 0x1) != 0 {
                self.rend_poly[..opaque_count].sort_by(|a, b| {
                    a.translucent
                        .cmp(&b.translucent)
                        .then(a.bottom_y.cmp(&b.bottom_y))
                        .then(a.top_y.cmp(&b.top_y))
                });
            } else {
                let index = self.rend_poly_count as usize;
                self.rend_poly[..index].sort_by(|a, b| {
                    a.translucent
                        .cmp(&b.translucent)
                        .then(a.bottom_y.cmp(&b.bottom_y))
                        .then(a.top_y.cmp(&b.top_y))
                });
            }
        }
        self.swap_buffers = false;
    }

    // Moved to struct Emulator method (Because use emulator method)
    // pub fn check_fifo_irq(&mut self);
    // pub fn write_gxfifo(&mut self, word: u32, cmd_param_amounts: &[u8; 256]);
    // pub fn write_fifo_direct(&mut self, address: u32, word: u32);

    pub fn get_disp3dcnt(&self) -> u16 {
        self.disp3dcnt.get()
    }

    pub fn get_gxstat(&self) -> u32 {
        let mut reg: u32 = 0;

        reg |= self.gxstat.box_pos_vec_busy as u32;
        reg |= (self.gxstat.boxtest_result as u32) << 1;
        reg |= ((self.model_view_sp & 0x1F) as u32) << 8;
        reg |= (self.gxstat.mtx_stack_busy as u32) << 14;
        reg |= (self.gxfifo.len() << 16) as u32;
        reg |= ((self.gxfifo.len() < 128) as u32) << 25;
        reg |= ((self.gxfifo.is_empty()) as u32) << 26;
        reg |= (self.gxstat.geo_busy as u32) << 27;
        reg |= (self.gxstat.gxfifo_irq_stat as u32) << 30;

        reg
    }

    pub fn get_vert_count(&self) -> u16 {
        self.geo_vert_count as u16
    }

    pub fn get_poly_count(&self) -> u16 {
        self.geo_poly_count as u16
    }

    pub fn read_clip_mtx(&mut self, address: u32) -> u32 {
        self.update_clip_matrix();
        let x = ((address - 0x04000640) % 4) as usize;
        let y = ((address - 0x04000640) / 4) as usize;
        self.clip_mtx.m[y][x] as u32
    }

    pub fn read_vec_mtx(&self, address: u32) -> u32 {
        let addr = address - 0x04000680;
        let x = (addr % 3) as usize;
        let y = (addr / 3) as usize;
        self.vector_mtx.m[y][x] as u32
    }

    pub fn read_vec_test(&self, address: u32) -> u16 {
        self.vec_test_result[((address - 0x04000630) / 2) as usize] as u16
    }

    pub fn set_disp3dcnt(&mut self, halfword: u16) {
        self.disp3dcnt.texture_mapping = (halfword & 1) != 0;
        self.disp3dcnt.highlight_shading = (halfword & (1 << 1)) != 0;
        self.disp3dcnt.alpha_test = (halfword & (1 << 2)) != 0;
        self.disp3dcnt.alpha_blending = (halfword & (1 << 3)) != 0;
        self.disp3dcnt.anti_aliasing = (halfword & (1 << 4)) != 0;
        self.disp3dcnt.edge_marking = (halfword & (1 << 5)) != 0;
        self.disp3dcnt.fog_color_mode = (halfword & (1 << 6)) != 0;
        self.disp3dcnt.fog_enable = (halfword & (1 << 7)) != 0;
        self.disp3dcnt.fog_depth_shift = ((halfword >> 8) & 0xF) as i32;

        //TODO: Underflow/overflow: check me
        self.disp3dcnt.color_buffer_underflow = (halfword & (1 << 12)) == 0;
        self.disp3dcnt.ram_overflow = (halfword & (1 << 13)) == 0;
        self.disp3dcnt.rear_plane_mode = (halfword & (1 << 14)) != 0;
    }

    pub fn set_clear_color(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("Set clear_color: {word:08X}");
        self.clear_color = word;
    }

    pub fn set_clear_depth(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("Set clear_depth: {word:08X}");
        self.clear_depth = word & 0x7FFF;
    }

    pub fn set_mtx_mode(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("Set mtx_mode: {word:08X}");
        self.mtx_mode = (word & 0x3) as u8;
    }

    pub fn mtx_push(&mut self) {
        #[cfg(feature = "tracing")]
        tracing::debug!("MTX_PUSH");

        match self.mtx_mode {
            0 => {
                self.projection_stack.set(&self.projection_mtx);
            }
            1 | 2 => {
                #[cfg(feature = "tracing")]
                let model_view_sp = self.model_view_sp as usize;
                tracing::debug!("self.model_view SP: {model_view_sp:02X}");

                if model_view_sp < 0x1F {
                    self.modelview_stack[model_view_sp].set(&self.modelview_mtx);
                    self.vector_stack[model_view_sp].set(&self.vector_mtx);
                } else {
                    #[cfg(feature = "tracing")]
                    tracing::error!("MTX_PUSH overflow!");
                    self.gxstat.mtx_overflow = true;
                }
                self.model_view_sp = (self.model_view_sp + 1) & 0x1F;
            }
            3 => {
                self.texture_stack.set(&self.texture_mtx);
            }
            unknown => {
                #[cfg(feature = "tracing")]
                tracing::error!("Unrecognized MTX_MODE {unknown}  for self.mtx_pop.");
            }
        }
    }

    pub fn mtx_pop(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::trace!("self.mtx_pop: {word:08X}");

        let offset = ((word & 0x3F) << 2) >> 2;
        match self.mtx_mode {
            2 => {
                self.model_view_sp -= offset as u8;
                self.modelview_mtx
                    .set(&self.modelview_stack[(self.model_view_sp & 0x1F) as usize]);
            }
            unknown => {
                #[cfg(feature = "tracing")]
                tracing::error!("Unrecognized MTX_MODE {unknown}  for self.mtx_pop.");
            }
        }
    }

    pub fn mtx_identity(&mut self) {
        #[cfg(feature = "tracing")]
        tracing::trace!("mtx_identity");

        match self.mtx_mode {
            0 => self.projection_mtx.set_identity(),
            1 => self.modelview_mtx.set_identity(),
            2 => {
                self.modelview_mtx.set_identity();
                self.vector_mtx.set_identity();
            }
            3 => self.texture_mtx.set_identity(),
            unknown => {
                #[cfg(feature = "tracing")]
                tracing::error!("Unrecognized MTX_MODE {unknown} in mtx_identity.");
            }
        }
    }

    pub fn mtx_mult_4x4(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("MTX_MULT_4x4: {word:08X}");

        self.add_mult_param(word);

        if self.mult_params_index >= 16 {
            self.mtx_mult(false);
        }
    }

    pub fn mtx_mult_4x3(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("MTX_MULT_4x3: {word:08X}");

        self.add_mult_param(word);

        if (self.mult_params_index & 0x3) == 0x3 {
            self.mult_params_index += 1;
        }

        if self.mult_params_index >= 16 {
            self.mtx_mult(true);
        }
    }

    pub fn mtx_mult_3x3(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("MTX_MULT_3x3: {word:08X}");

        self.add_mult_param(word);

        if (self.mult_params_index & 0x3) == 0x3 {
            self.mult_params_index += 1;
        }

        if self.mult_params_index >= 11 {
            self.mtx_mult(true);
        }
    }

    pub fn mtx_trans(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("MTX_TRANS: {word:08X}");

        if self.mult_params_index == 0 {
            self.mult_params_index = 12;
        }

        self.add_mult_param(word);

        if self.mult_params_index >= 15 {
            self.mtx_mult(true);
        }
    }

    pub fn color(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("COLOR: {word:08X}");
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

        // Front face
        face[0] = cube[0].clone();
        face[1] = cube[1].clone();
        face[2] = cube[2].clone();
        face[3] = cube[3].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
            return;
        }

        // Back face
        face[0] = cube[4].clone();
        face[1] = cube[5].clone();
        face[2] = cube[6].clone();
        face[3] = cube[7].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
            return;
        }

        // Left face
        face[0] = cube[0].clone();
        face[1] = cube[3].clone();
        face[2] = cube[4].clone();
        face[3] = cube[5].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
            return;
        }

        // Right face
        face[0] = cube[1].clone();
        face[1] = cube[2].clone();
        face[2] = cube[7].clone();
        face[3] = cube[6].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
            return;
        }

        // Bottom face
        face[0] = cube[0].clone();
        face[1] = cube[1].clone();
        face[2] = cube[6].clone();
        face[3] = cube[5].clone();
        vertices = self.clip(&mut face, 4, 0, false);
        if vertices > 0 {
            self.gxstat.boxtest_result = true;
            return;
        }

        // Top face
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
