//! ARM CPU core
// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp
//!
use crate::Emulator;
use lunaris_ds_gpu::gpu_3d::consts::{CMD_CYCLE_AMOUNTS, CMD_PARAM_AMOUNTS};
use lunaris_ds_gpu::gpu_3d::structs::Matrix;

impl Emulator {
    pub fn exec_command(&mut self) {
        if let Some(cmd) = self.read_command() {
            let param = cmd.param;
            let cmd_index = cmd.command as usize;
            let param_index = self.gpu.engine_3d.cmd_param_count as usize;

            self.gpu.engine_3d.cmd_params[param_index] = param;
            self.gpu.engine_3d.cmd_param_count += 1;

            // Execute command once all parameters are available.
            if self.gpu.engine_3d.cmd_param_count >= CMD_PARAM_AMOUNTS[cmd_index] {
                self.gpu.engine_3d.cycles += CMD_CYCLE_AMOUNTS[cmd_index] as i64;

                // Update GEO_BUSY flag:
                self.gpu.engine_3d.gxstat.geo_busy =
                    self.gpu.engine_3d.cycles > 0 && !self.gpu.engine_3d.gxpipe.is_empty();

                match cmd_index {
                    0x00 => (),
                    0x10 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("Command Parameter: {}", self.gpu.engine_3d.cmd_params[0]);

                        self.gpu.engine_3d.mtx_mode =
                            (self.gpu.engine_3d.cmd_params[0] & 0x3) as u8;
                    }
                    0x11 => self.gpu.engine_3d.mtx_push(),
                    0x12 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("Command Parameter: {}", self.gpu.engine_3d.cmd_params[0]);

                        if self.gpu.engine_3d.mtx_mode != 3 {
                            self.gpu.engine_3d.clip_dirty = true;
                        }
                        match self.gpu.engine_3d.mtx_mode {
                            0 => {
                                let stack_matrix = &self.gpu.engine_3d.projection_stack;
                                self.gpu.engine_3d.projection_mtx.set(stack_matrix);
                            }
                            1 => {}
                            2 => {
                                let offset =
                                    (((self.gpu.engine_3d.cmd_params[0] & 0x3F) << 2) >> 2) as u8;
                                self.gpu.engine_3d.model_view_sp -= offset;

                                if self.gpu.engine_3d.model_view_sp >= 0x1F {
                                    #[cfg(feature = "tracing")]
                                    tracing::info!("MTX_POP overflow!");
                                    self.gpu.engine_3d.gxstat.mtx_overflow = true;
                                } else {
                                    let stack_index =
                                        (self.gpu.engine_3d.model_view_sp & 0x1F) as usize;
                                    self.gpu
                                        .engine_3d
                                        .modelview_mtx
                                        .set(&self.gpu.engine_3d.modelview_stack[stack_index]);
                                    self.gpu
                                        .engine_3d
                                        .vector_mtx
                                        .set(&self.gpu.engine_3d.vector_stack[stack_index]);
                                }
                                self.gpu.engine_3d.model_view_sp &= 0x3F;
                            }
                            3 => self
                                .gpu
                                .engine_3d
                                .texture_mtx
                                .set(&self.gpu.engine_3d.texture_stack),
                            _ => {
                                #[cfg(feature = "tracing")]
                                tracing::error!(
                                    "Unrecognized MTX_MODE {} for MTX_POP",
                                    self.gpu.engine_3d.mtx_mode
                                );
                            }
                        }
                    }
                    0x13 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("MTX_STORE");

                        match self.gpu.engine_3d.mtx_mode {
                            0 => self
                                .gpu
                                .engine_3d
                                .projection_stack
                                .set(&self.gpu.engine_3d.projection_mtx),
                            1 => {}
                            2 => {
                                let offset = self.gpu.engine_3d.cmd_params[0] & 0x1F;
                                if offset < 31 {
                                    self.gpu.engine_3d.modelview_stack[offset as usize]
                                        .set(&self.gpu.engine_3d.modelview_mtx);
                                    self.gpu.engine_3d.vector_stack[offset as usize]
                                        .set(&self.gpu.engine_3d.vector_mtx);
                                } else {
                                    #[cfg(feature = "tracing")]
                                    tracing::info!("MTX_POP overflow!");
                                    self.gpu.engine_3d.gxstat.mtx_overflow = true;
                                }
                            }
                            3 => self
                                .gpu
                                .engine_3d
                                .texture_mtx
                                .set(&self.gpu.engine_3d.texture_stack),
                            _ => {
                                #[cfg(feature = "tracing")]
                                tracing::error!(
                                    "Unrecognized MTX_MODE {} for MTX_POP",
                                    self.gpu.engine_3d.mtx_mode
                                );
                            }
                        }
                    }
                    0x14 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("MTX_RESTORE {}", self.gpu.engine_3d.cmd_params[0] & 0xFF);

                        if self.gpu.engine_3d.mtx_mode != 3 {
                            self.gpu.engine_3d.clip_dirty = true;
                        }
                        match self.gpu.engine_3d.mtx_mode {
                            0 => self
                                .gpu
                                .engine_3d
                                .projection_stack
                                .set(&self.gpu.engine_3d.projection_mtx),
                            1 => {}
                            2 => {
                                let offset = self.gpu.engine_3d.cmd_params[0] & 0x1F;
                                if offset < 31 {
                                    self.gpu.engine_3d.modelview_stack[offset as usize]
                                        .set(&self.gpu.engine_3d.modelview_mtx);
                                    self.gpu.engine_3d.vector_stack[offset as usize]
                                        .set(&self.gpu.engine_3d.vector_mtx);
                                } else {
                                    #[cfg(feature = "tracing")]
                                    tracing::info!("MTX_RESTORE overflow!");
                                    self.gpu.engine_3d.gxstat.mtx_overflow = true;
                                }
                            }
                            _ => {
                                #[cfg(feature = "tracing")]
                                tracing::error!(
                                    "Unrecognized MTX_MODE {} for MTX_RESTORE",
                                    self.gpu.engine_3d.mtx_mode
                                );
                            }
                        }
                    }
                    0x15 => {
                        if self.gpu.engine_3d.mtx_mode != 3 {
                            self.gpu.engine_3d.clip_dirty = true;
                        }
                        self.gpu.engine_3d.mtx_identity();
                    }
                    0x16 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("MTX_LOAD_4x4");

                        if self.gpu.engine_3d.mtx_mode != 3 {
                            self.gpu.engine_3d.clip_dirty = true;
                        }

                        // NOTE: current_matrix is unused!
                        let mut current_mtx = &mut Matrix::zeros();
                        match self.gpu.engine_3d.mtx_mode {
                            0 => current_mtx = &mut self.gpu.engine_3d.projection_mtx,
                            1 => current_mtx = &mut self.gpu.engine_3d.modelview_mtx,
                            2 => {
                                current_mtx = &mut self.gpu.engine_3d.modelview_mtx;
                                // for i in 0..4 {
                                //     for j in 0..4 {
                                //         let idx = (i * 4) + j;
                                //         self.gpu.engine_3d.vector_mtx.m[i][j] = self.gpu.engine_3d.cmd_params[idx] as i32;
                                //     }
                                // }
                                self.gpu
                                    .engine_3d
                                    .cmd_params
                                    .iter()
                                    .map(|&x| x as i32)
                                    .collect::<Vec<_>>()
                                    .chunks_exact(4)
                                    .enumerate()
                                    .for_each(|(i, row)| {
                                        self.gpu.engine_3d.vector_mtx.m[i].copy_from_slice(row);
                                    });
                            }
                            3 => current_mtx = &mut self.gpu.engine_3d.texture_mtx,
                            _ => {
                                #[cfg(feature = "tracing")]
                                tracing::error!(
                                    "Unrecognized MTX_MODE {} in MTX_LOAD_4x4",
                                    self.gpu.engine_3d.mtx_mode
                                );
                            }
                        }
                        for i in 0..4 {
                            for j in 0..4 {
                                let idx = (i * 4) + j;
                                current_mtx.m[i][j] = self.gpu.engine_3d.cmd_params[idx] as i32;
                            }
                        }
                    }
                    0x17 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("MTX_LOAD_4x3");

                        if self.gpu.engine_3d.mtx_mode != 3 {
                            self.gpu.engine_3d.clip_dirty = true;
                        }

                        match self.gpu.engine_3d.mtx_mode {
                            1 => {}
                            2 => {
                                self.gpu.engine_3d.vector_mtx.m[0][0] =
                                    self.gpu.engine_3d.cmd_params[0] as i32;
                                self.gpu.engine_3d.vector_mtx.m[0][1] =
                                    self.gpu.engine_3d.cmd_params[1] as i32;
                                self.gpu.engine_3d.vector_mtx.m[0][2] =
                                    self.gpu.engine_3d.cmd_params[2] as i32;

                                self.gpu.engine_3d.vector_mtx.m[1][0] =
                                    self.gpu.engine_3d.cmd_params[3] as i32;
                                self.gpu.engine_3d.vector_mtx.m[1][1] =
                                    self.gpu.engine_3d.cmd_params[4] as i32;
                                self.gpu.engine_3d.vector_mtx.m[1][2] =
                                    self.gpu.engine_3d.cmd_params[5] as i32;

                                self.gpu.engine_3d.vector_mtx.m[2][0] =
                                    self.gpu.engine_3d.cmd_params[6] as i32;
                                self.gpu.engine_3d.vector_mtx.m[2][1] =
                                    self.gpu.engine_3d.cmd_params[7] as i32;
                                self.gpu.engine_3d.vector_mtx.m[2][2] =
                                    self.gpu.engine_3d.cmd_params[8] as i32;

                                self.gpu.engine_3d.vector_mtx.m[3][0] =
                                    self.gpu.engine_3d.cmd_params[9] as i32;
                                self.gpu.engine_3d.vector_mtx.m[3][1] =
                                    self.gpu.engine_3d.cmd_params[10] as i32;
                                self.gpu.engine_3d.vector_mtx.m[3][2] =
                                    self.gpu.engine_3d.cmd_params[11] as i32;
                            }
                            3 => {}
                            _ => {
                                #[cfg(feature = "tracing")]
                                tracing::error!(
                                    "Unrecognized MTX_MODE {} in MTX_LOAD_4x3",
                                    self.gpu.engine_3d.mtx_mode
                                );
                            }
                        }
                    }
                    0x18 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("MTX_MULT_4x4");
                        let cmd_pointer = 0;
                        for i in 0..4 {
                            for j in 0..4 {
                                self.gpu.engine_3d.mult_params.m[i][j] =
                                    self.gpu.engine_3d.cmd_params[cmd_pointer] as i32;
                            }
                        }
                        self.gpu.engine_3d.mtx_mult(true);
                    }
                    0x19 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("MTX_MULT_4x3");
                        self.gpu.engine_3d.mult_params.m[0][0] =
                            self.gpu.engine_3d.cmd_params[0] as i32;
                        self.gpu.engine_3d.mult_params.m[0][1] =
                            self.gpu.engine_3d.cmd_params[1] as i32;
                        self.gpu.engine_3d.mult_params.m[0][2] =
                            self.gpu.engine_3d.cmd_params[2] as i32;

                        self.gpu.engine_3d.mult_params.m[1][0] =
                            self.gpu.engine_3d.cmd_params[3] as i32;
                        self.gpu.engine_3d.mult_params.m[1][1] =
                            self.gpu.engine_3d.cmd_params[4] as i32;
                        self.gpu.engine_3d.mult_params.m[1][2] =
                            self.gpu.engine_3d.cmd_params[5] as i32;

                        self.gpu.engine_3d.mult_params.m[2][0] =
                            self.gpu.engine_3d.cmd_params[6] as i32;
                        self.gpu.engine_3d.mult_params.m[2][1] =
                            self.gpu.engine_3d.cmd_params[7] as i32;
                        self.gpu.engine_3d.mult_params.m[2][2] =
                            self.gpu.engine_3d.cmd_params[8] as i32;

                        self.gpu.engine_3d.mult_params.m[3][0] =
                            self.gpu.engine_3d.cmd_params[9] as i32;
                        self.gpu.engine_3d.mult_params.m[3][1] =
                            self.gpu.engine_3d.cmd_params[10] as i32;
                        self.gpu.engine_3d.mult_params.m[3][2] =
                            self.gpu.engine_3d.cmd_params[11] as i32;

                        self.gpu.engine_3d.mtx_mult(true);
                    }
                    0x1A => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("MTX_MULT_3x3");
                        self.gpu.engine_3d.mult_params.m[0][0] =
                            self.gpu.engine_3d.cmd_params[0] as i32;
                        self.gpu.engine_3d.mult_params.m[0][1] =
                            self.gpu.engine_3d.cmd_params[1] as i32;
                        self.gpu.engine_3d.mult_params.m[0][2] =
                            self.gpu.engine_3d.cmd_params[2] as i32;

                        self.gpu.engine_3d.mult_params.m[1][0] =
                            self.gpu.engine_3d.cmd_params[3] as i32;
                        self.gpu.engine_3d.mult_params.m[1][1] =
                            self.gpu.engine_3d.cmd_params[4] as i32;
                        self.gpu.engine_3d.mult_params.m[1][2] =
                            self.gpu.engine_3d.cmd_params[5] as i32;

                        self.gpu.engine_3d.mult_params.m[2][0] =
                            self.gpu.engine_3d.cmd_params[6] as i32;
                        self.gpu.engine_3d.mult_params.m[2][1] =
                            self.gpu.engine_3d.cmd_params[7] as i32;
                        self.gpu.engine_3d.mult_params.m[2][2] =
                            self.gpu.engine_3d.cmd_params[8] as i32;

                        self.gpu.engine_3d.mtx_mult(true);
                    }
                    0x1B => {
                        #[cfg(feature = "tracing")]
                        tracing::info!(
                            "MTX_SCALE: {}, {}, {}",
                            self.gpu.engine_3d.cmd_params[0],
                            self.gpu.engine_3d.cmd_params[1],
                            self.gpu.engine_3d.cmd_params[2]
                        );
                        self.gpu.engine_3d.mult_params.m[0][0] =
                            self.gpu.engine_3d.cmd_params[0] as i32;
                        self.gpu.engine_3d.mult_params.m[1][1] =
                            self.gpu.engine_3d.cmd_params[1] as i32;
                        self.gpu.engine_3d.mult_params.m[2][2] =
                            self.gpu.engine_3d.cmd_params[2] as i32;
                        self.gpu.engine_3d.mtx_mult(false);
                    }
                    0x1C => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("MTX_TRANS");
                        self.gpu.engine_3d.mult_params.m[3][0] =
                            self.gpu.engine_3d.cmd_params[0] as i32;
                        self.gpu.engine_3d.mult_params.m[3][1] =
                            self.gpu.engine_3d.cmd_params[1] as i32;
                        self.gpu.engine_3d.mult_params.m[3][2] =
                            self.gpu.engine_3d.cmd_params[2] as i32;
                        self.gpu.engine_3d.mtx_mult(true);
                    }
                    0x20 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("COLOR: {}", self.gpu.engine_3d.cmd_params[0]);
                        self.gpu.engine_3d.current_color = self.gpu.engine_3d.cmd_params[0];
                    }
                    0x21 => {
                        self.gpu.engine_3d.normal();
                    }
                    0x22 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("CTEXCOORD: {}", self.gpu.engine_3d.cmd_params[0]);
                        self.gpu.engine_3d.current_texcoords[0] =
                            (self.gpu.engine_3d.cmd_params[0] & 0xFFFF) as i16;
                        self.gpu.engine_3d.current_texcoords[1] =
                            (self.gpu.engine_3d.cmd_params[0] >> 16) as i16;

                        if self.gpu.engine_3d.teximage_param.transformation_mode == 1 {
                            let texcoords = [
                                self.gpu.engine_3d.cmd_params[0] & 0xFFFF,
                                self.gpu.engine_3d.cmd_params[0] >> 16,
                            ];

                            for i in 0..2 {
                                self.gpu.engine_3d.current_texcoords[i] = ((texcoords[0] as i32
                                    * self.gpu.engine_3d.texture_mtx.m[0][i]
                                    + texcoords[1] as i32 * self.gpu.engine_3d.texture_mtx.m[1][i]
                                    + self.gpu.engine_3d.texture_mtx.m[2][i]
                                    + self.gpu.engine_3d.texture_mtx.m[3][i])
                                    >> 12)
                                    as i16;
                            }
                        } else {
                            self.gpu.engine_3d.current_texcoords[0] =
                                (self.gpu.engine_3d.cmd_params[0] & 0xFFFF) as i16;
                            self.gpu.engine_3d.current_texcoords[1] =
                                (self.gpu.engine_3d.cmd_params[0] >> 16) as i16;
                        }
                    }
                    0x23 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!(
                            "VTX_16: {}, {}",
                            self.gpu.engine_3d.cmd_params[0],
                            self.gpu.engine_3d.cmd_params[1]
                        );
                        self.gpu.engine_3d.current_vertex[0] =
                            (self.gpu.engine_3d.cmd_params[0] & 0xFFFF) as i16;
                        self.gpu.engine_3d.current_vertex[1] =
                            (self.gpu.engine_3d.cmd_params[0] >> 16) as i16;
                        self.gpu.engine_3d.current_vertex[2] =
                            (self.gpu.engine_3d.cmd_params[1] & 0xFFFF) as i16;
                        self.gpu.engine_3d.add_vertex();
                    }
                    0x24 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("VTX_10");
                        self.gpu.engine_3d.current_vertex[0] =
                            ((self.gpu.engine_3d.cmd_params[0] & 0x000003FF) << 6) as i16;
                        self.gpu.engine_3d.current_vertex[1] =
                            ((self.gpu.engine_3d.cmd_params[0] & 0x000FFC00) >> 4) as i16;
                        self.gpu.engine_3d.current_vertex[2] =
                            ((self.gpu.engine_3d.cmd_params[0] & 0x3FF00000) >> 14) as i16;
                        self.gpu.engine_3d.add_vertex();
                    }
                    0x25 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("VTX_XY");
                        self.gpu.engine_3d.current_vertex[0] =
                            (self.gpu.engine_3d.cmd_params[0] & 0xFFFF) as i16;
                        self.gpu.engine_3d.current_vertex[1] =
                            (self.gpu.engine_3d.cmd_params[0] >> 16) as i16;
                        self.gpu.engine_3d.add_vertex();
                    }
                    0x26 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("VTX_XZ");
                        self.gpu.engine_3d.current_vertex[0] =
                            (self.gpu.engine_3d.cmd_params[0] & 0xFFFF) as i16;
                        self.gpu.engine_3d.current_vertex[2] =
                            (self.gpu.engine_3d.cmd_params[0] >> 16) as i16;
                        self.gpu.engine_3d.add_vertex();
                    }
                    0x27 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("VTX_YZ");
                        self.gpu.engine_3d.current_vertex[1] =
                            (self.gpu.engine_3d.cmd_params[0] & 0xFFFF) as i16;
                        self.gpu.engine_3d.current_vertex[2] =
                            (self.gpu.engine_3d.cmd_params[0] >> 16) as i16;
                        self.gpu.engine_3d.add_vertex();
                    }
                    0x28 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("VTX_DIFF");
                        self.gpu.engine_3d.current_vertex[2] =
                            ((self.gpu.engine_3d.cmd_params[0] & 0x000003FF) << 6) as i16;
                        self.gpu.engine_3d.current_vertex[1] =
                            ((self.gpu.engine_3d.cmd_params[0] & 0x000FFC00) >> 4) as i16;
                        self.gpu.engine_3d.current_vertex[2] =
                            ((self.gpu.engine_3d.cmd_params[0] >> 16 & 0x3FF00000) >> 14) as i16;
                        self.gpu.engine_3d.add_vertex();
                    }
                    0x29 => {
                        self.gpu
                            .engine_3d
                            .set_polygon_attr(self.gpu.engine_3d.cmd_params[0]);
                    }
                    0x2A => {
                        self.gpu
                            .engine_3d
                            .set_teximage_param(self.gpu.engine_3d.cmd_params[0]);
                    }
                    0x2B => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("PLTT_BASE: {}", self.gpu.engine_3d.cmd_params[0]);
                        self.gpu.engine_3d.pltt_base = self.gpu.engine_3d.cmd_params[0] & 0x1FFF;
                    }
                    0x30 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("VTX_DIFF");

                        self.gpu.engine_3d.diffuse_color =
                            (self.gpu.engine_3d.cmd_params[0] & 0x7FFF) as u16;
                        self.gpu.engine_3d.ambient_color =
                            ((self.gpu.engine_3d.cmd_params[0] >> 16) & 0x7FFF) as u16;
                        let shift = self.gpu.engine_3d.cmd_params[0] & (1 << 15);
                        if shift != 0 {
                            self.gpu.engine_3d.current_color =
                                self.gpu.engine_3d.diffuse_color as u32;
                        }
                    }
                    0x31 => {
                        //             //printf("\nSPE_EMI");
                        self.gpu.engine_3d.specular_color =
                            (self.gpu.engine_3d.cmd_params[0] & 0x7FFF) as u16;
                        self.gpu.engine_3d.emission_color =
                            ((self.gpu.engine_3d.cmd_params[0] >> 16) & 0x7FFF) as u16;
                        self.gpu.engine_3d.using_shine_table =
                            (self.gpu.engine_3d.cmd_params[0] & (1 << 15)) != 0;
                    }
                    0x32 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("LIGHT_VECTOR");

                        let light_vector = [
                            ((self.gpu.engine_3d.cmd_params[0] & 0x3FF) << 6) >> 6,
                            (((self.gpu.engine_3d.cmd_params[0] >> 10) & 0x3FF) << 6) >> 6,
                            (((self.gpu.engine_3d.cmd_params[0] >> 20) & 0x3FF) << 6) >> 6,
                        ];
                        let light_vector_i32 = light_vector.map(|x| x as i32);
                        let index = (self.gpu.engine_3d.cmd_params[0] >> 30) as usize;

                        for i in 0..3 {
                            self.gpu.engine_3d.light_direction[index][i] =
                                ((light_vector_i32[0] * self.gpu.engine_3d.vector_mtx.m[0][i]
                                    + light_vector_i32[1] * self.gpu.engine_3d.vector_mtx.m[1][i]
                                    + light_vector_i32[2] * self.gpu.engine_3d.vector_mtx.m[2][i])
                                    >> 12) as i16;
                        }
                    }
                    0x33 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("LIGHT_COLOR: {}", self.gpu.engine_3d.cmd_params[0]);
                        let index = (self.gpu.engine_3d.cmd_params[0] >> 30) as usize;
                        self.gpu.engine_3d.light_color[index] =
                            (self.gpu.engine_3d.cmd_params[0] & 0x7FF) as u16;
                    }
                    0x34 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("SHININESS");
                        for i in 0..32 {
                            let index = i * 4;
                            self.gpu.engine_3d.shine_table[index] =
                                (self.gpu.engine_3d.cmd_params[i] & 0xFF) as u8;
                            self.gpu.engine_3d.shine_table[index + 1] =
                                ((self.gpu.engine_3d.cmd_params[i] >> 8) & 0xFF) as u8;
                            self.gpu.engine_3d.shine_table[index + 2] =
                                ((self.gpu.engine_3d.cmd_params[i] >> 16) & 0xFF) as u8;
                            self.gpu.engine_3d.shine_table[index + 3] =
                                (self.gpu.engine_3d.cmd_params[i] >> 24) as u8;
                        }
                    }
                    0x40 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("BEGIN_VTXS");
                        self.gpu.engine_3d.polygon_type = self.gpu.engine_3d.cmd_params[0] & 0x3;
                        self.gpu.engine_3d.current_poly_attr =
                            self.gpu.engine_3d.polygon_attr.clone();
                        self.gpu.engine_3d.consecutive_polygons = 0;
                        self.gpu.engine_3d.vertex_list_count = 0;
                    }
                    0x41 => {
                        #[cfg(feature = "tracing")]
                        tracing::info!("END_VTXS");
                    }
                    0x50 => {
                        let param = self.gpu.engine_3d.cmd_params[0];
                        self.gpu.engine_3d.swap_buffers(param)
                    }
                    0x60 => {
                        let param = self.gpu.engine_3d.cmd_params[0];
                        self.gpu.engine_3d.viewport(param)
                    }
                    0x70 => self.gpu.engine_3d.box_test(),
                    0x72 => self.gpu.engine_3d.vec_test(),
                    _ => {
                        #[cfg(feature = "tracing")]
                        tracing::error!("Unrecognized GXFIFO command: {}", cmd.command);
                    }
                }
                self.gpu.engine_3d.cmd_param_count = 0;
            }
        }
    }
}
