// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry engine: executes GX commands.
//!
//! ```text
//!  GX command --> mod.rs (execute_command: matrix stack, vertex state)
//!                   |-- matrix.rs   MTX_* math
//!                   |-- lighting.rs NORMAL -> vertex color
//!                   `-- polygon.rs  VTX_* -> polygons (culling, viewport)
//!                          `-- clip.rs  view-volume clipping (+ BOX_TEST)
//!  polygon RAM (double buffered) --> swapped at VBlank --> raster/
//! ```
//!
//! Ported from melonDS `GPU3D.cpp` (GPL-3.0-or-later); see also GBATEK
//! "DS 3D Matrix Load/Multiply", "DS 3D Polygon Definitions" and
//! "DS 3D Polygon Light Parameters".
//!
//! ## Matrices and the matrix stack
//!
//! ```text
//!   MTX_MODE 0  projection          stack: 1 entry
//!            1  position            stack: 31 entries  (position and vector
//!            2  position + vector          matrices are pushed/popped together)
//!            3  texture             stack: 1 entry
//!
//!   mode 2 is used for models with lighting: every MTX_* also updates the
//!   vector matrix that rotates normals and light directions.
//!
//!   clip matrix = position * projection   (recomputed when either changes)
//!
//!   MTX_PUSH:  stack [ M0 ][ M1 ][ M2 ][    ]      pointer 3 -> 4
//!                                    ^ current matrix copied here
//!   MTX_POP n: pointer -= n, current = stack[pointer]
//!   MTX_STORE / MTX_RESTORE n: copy to / from slot n without moving the pointer
//!   overflow / underflow -> GXSTAT bit 15 (error flag)
//!
//!   a typical frame:
//!     MTX_MODE 0, MTX_LOAD_4x4 (perspective)       projection once
//!     MTX_MODE 2, MTX_IDENTITY, camera transform   then per object:
//!       MTX_PUSH, MTX_TRANS / MTX_MULT_3x3 (place + rotate the object),
//!       BEGIN_VTXS, VTX_*..., MTX_POP 1
//! ```
//!
//! GBATEK: [DS 3D Matrix Stack](https://problemkaputt.de/gbatek.htm#ds3dmatrixstack),
//! [DS 3D Matrix Types](https://problemkaputt.de/gbatek.htm#ds3dmatrixtypes)

mod clip;
mod lighting;
mod matrix;
mod polygon;

use matrix::{mtx_mult_4x4, transform};

use crate::engine_3d::state::{Gpu3D, IDENTITY, MAX_POLYGONS, MAX_VERTICES, Mtx, Polygon, Vertex};

/// Sign-extends a 10-bit field (bits `shift..shift+10` of `v`).
#[inline]
const fn sext10(v: u32, shift: u32) -> i16 {
    // DS 3D Polygon Light Parameters: https://problemkaputt.de/gbatek.htm#ds3dpolygonlightparameters
    ((((v >> shift) & 0x3FF) << 6) as u16 as i16) >> 6 // sign-extend a 10-bit field (s.9 fixed point)
}

/// 32x32 -> 64-bit product (fixed-point helper).
#[inline]
pub(super) fn mul(a: i32, b: i32) -> i64 {
    i64::from(a) * i64::from(b)
}

impl Default for Gpu3D {
    fn default() -> Self {
        Self::new()
    }
}

impl Gpu3D {
    /// Geometry engine with identity matrices and the reset viewport.
    pub fn new() -> Self {
        let mut s = Self {
            cycles: 0,
            gxfifo: Default::default(),
            gxpipe: Default::default(),
            cmd_params: [0; 32],
            cmd_param_count: 0,
            param_count: 0,
            cmd_count: 0,
            total_params: 0,
            current_cmd: 0,
            disp_cnt: 0,
            alpha_ref_val: 0,
            clear_attr1: 0,
            clear_attr2: 0,
            fog_color: 0,
            fog_offset: 0,
            fog_density: [0; 32],
            edge_table: [0; 8],
            toon_table: [0; 32],
            zero_dot_w_limit: 0,
            gxstat: 0,
            matrix_mode: 0,
            proj_matrix: IDENTITY,
            pos_matrix: IDENTITY,
            vec_matrix: IDENTITY,
            tex_matrix: IDENTITY,
            clip_matrix: IDENTITY,
            clip_matrix_dirty: true,
            proj_stack: IDENTITY,
            tex_stack: IDENTITY,
            pos_stack: [IDENTITY; 32],
            vec_stack: [IDENTITY; 32],
            proj_stack_ptr: 0,
            tex_stack_ptr: 0,
            pos_stack_ptr: 0,
            viewport: Default::default(),
            polygon_attr: 0,
            cur_polygon_attr: 0,
            tex_param: 0,
            tex_palette: 0,
            polygon_mode: 0,
            cur_vertex: [0; 3],
            vertex_color: [31; 3],
            normal: [0; 3],
            raw_texcoords: [0; 2],
            texcoords: [0; 2],
            light_direction: [[0; 3]; 4],
            light_color: [[0; 3]; 4],
            spec_recip: [0; 4],
            mat_diffuse: [0; 3],
            mat_ambient: [0; 3],
            mat_specular: [0; 3],
            mat_emission: [0; 3],
            use_shininess_table: false,
            shininess_table: [0; 128],
            pos_test_result: [0; 4],
            vec_test_result: [0; 3],
            temp_vertices: [Vertex::default(); 4],
            vertex_num_in_poly: 0,
            num_consecutive_polygons: 0,
            last_strip_polygon: None,
            vertex_ram: [
                vec![Vertex::default(); MAX_VERTICES],
                vec![Vertex::default(); MAX_VERTICES],
            ],
            polygon_ram: [
                vec![Polygon::default(); MAX_POLYGONS],
                vec![Polygon::default(); MAX_POLYGONS],
            ],
            cur_bank: 0,
            num_vertices: 0,
            num_polygons: 0,
            num_opaque_polygons: 0,
            swap_buffers: false,
            flush_attributes: 0,
            render_polygons: Vec::with_capacity(MAX_POLYGONS),
            render_bank: 1,
            render: Default::default(),
            render_dirty: true,
            rend_poly_count: 0,
            rend_vert_count: 0,
            raster: Default::default(),
            hires: Default::default(),
        };
        // Viewport 0,0-255,191 (melonDS reset value).
        // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
        s.set_viewport(0xBFFF_0000); // VIEWPORT reset value: X1=0, Y1=0, X2=255, Y2=191
        s
    }

    /// Resets the geometry engine to its power-on state.
    pub fn power_on(&mut self) {
        *self = Self::new();
    }

    /// Recomputes the clip matrix (projection * position) when dirty.
    fn update_clip_matrix(&mut self) {
        if self.clip_matrix_dirty {
            self.clip_matrix = self.current_clip_matrix();
            self.clip_matrix_dirty = false;
        }
    }

    /// Projection * position matrix (CLIPMTX_RESULT).
    pub(crate) fn current_clip_matrix(&self) -> Mtx {
        if !self.clip_matrix_dirty {
            return self.clip_matrix;
        }
        let mut m = self.proj_matrix;
        mtx_mult_4x4(&mut m, &self.pos_matrix);
        m
    }

    /// VIEWPORT (0x60): x1,y1,x2,y2 with Y flipped to screen space.
    fn set_viewport(&mut self, param: u32) {
        let v = &mut self.viewport;
        // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
        v.x0 = (param & 0xFF) as i32; // X1 (bits 0-7)
        v.y0 = (191 - ((param >> 8) & 0xFF) as i32) & 0xFF; // Y1 (bits 8-15), flipped because hardware Y counts from the bottom
        v.x1 = ((param >> 16) & 0xFF) as i32; // X2 (bits 16-23)
        v.y1 = (191 - (param >> 24) as i32) & 0xFF; // Y2 (bits 24-31), flipped
        v.width = (v.x1 - v.x0 + 1) & 0x1FF; // width wraps at 9 bits
        v.height = (v.y0 - v.y1 + 1) & 0xFF; // height wraps at 8 bits
    }

    /// Executes one complete GX command whose parameters are in
    /// `cmd_params`.
    ///
    /// ```text
    /// from: ds_emu exec_command() once all parameters are collected
    ///   |
    ///   v
    /// execute_command(cmd)
    ///   0x10-0x1C  matrix mode, push/pop/store/restore, identity,
    ///              load/multiply 4x4/4x3/3x3, scale, translate
    ///   0x20-0x2B  color, normal (lighting), texcoord, VTX_16/10/XY/
    ///              XZ/YZ/DIFF -> submit_vertex(), polygon/texture attrs
    ///   0x30-0x34  material, light vectors/colors, shininess table
    ///   0x40-0x41  BEGIN_VTXS / END_VTXS
    ///   0x50       SWAP_BUFFERS (waits for VBlank: end_of_frame)
    ///   0x60       VIEWPORT
    ///   0x70-0x72  BOX_TEST, POS_TEST, VEC_TEST
    ///   |
    ///   v
    /// to:   submit_vertex() -> submit_polygon() -> polygon/vertex RAM of
    ///       the current bank
    /// ```
    #[expect(clippy::too_many_lines)]
    pub fn execute_command(&mut self, cmd: u8) {
        let p = self.cmd_params;
        let p0 = p[0];
        let params: [i32; 32] = p.map(|v| v as i32);
        let mode = self.matrix_mode;
        match cmd {
            // DS 3D Geometry Commands: https://problemkaputt.de/gbatek.htm#ds3dgeometrycommands
            0x10 => self.matrix_mode = p0 & 3, // MTX_MODE - Set Matrix Mode (bits 0-1)
            // DS 3D Matrix Stack: https://problemkaputt.de/gbatek.htm#ds3dmatrixstack
            0x11 => match mode {
                // MTX_PUSH - Push Current Matrix on Stack
                0 => {
                    if self.proj_stack_ptr > 0 {
                        // DS 3D Matrix Stack: https://problemkaputt.de/gbatek.htm#ds3dmatrixstack
                        self.gxstat |= 1 << 15; // GXSTAT bit 15: matrix stack overflow/underflow error
                    }
                    self.proj_stack = self.proj_matrix;
                    self.proj_stack_ptr = (self.proj_stack_ptr + 1) & 1;
                }
                3 => {
                    if self.tex_stack_ptr > 0 {
                        self.gxstat |= 1 << 15; // GXSTAT bit 15: matrix stack error
                    }
                    self.tex_stack = self.tex_matrix;
                    self.tex_stack_ptr = (self.tex_stack_ptr + 1) & 1;
                }
                _ => {
                    if self.pos_stack_ptr > 30 {
                        self.gxstat |= 1 << 15; // GXSTAT bit 15: matrix stack error
                    }
                    let i = (self.pos_stack_ptr & 0x1F) as usize; // position stack has 31 entries (index wraps at 32)
                    self.pos_stack[i] = self.pos_matrix;
                    self.vec_stack[i] = self.vec_matrix;
                    self.pos_stack_ptr = (self.pos_stack_ptr + 1) & 0x3F; // stack pointer is 6 bits; bit 5 is the overflow position
                }
            },
            0x12 => match mode {
                // MTX_POP - Pop Current Matrix from Stack
                0 => {
                    if self.proj_stack_ptr == 0 {
                        self.gxstat |= 1 << 15; // GXSTAT bit 15: matrix stack error
                    }
                    self.proj_stack_ptr = self.proj_stack_ptr.wrapping_sub(1) & 1;
                    self.proj_matrix = self.proj_stack;
                    self.clip_matrix_dirty = true;
                }
                3 => {
                    if self.tex_stack_ptr == 0 {
                        self.gxstat |= 1 << 15; // GXSTAT bit 15: matrix stack error
                    }
                    self.tex_stack_ptr = self.tex_stack_ptr.wrapping_sub(1) & 1;
                    self.tex_matrix = self.tex_stack;
                }
                _ => {
                    let offset = ((p0 << 26) as i32) >> 26; // MTX_POP: sign-extend 6-bit pop count
                    self.pos_stack_ptr =
                        (self.pos_stack_ptr as i32).wrapping_sub(offset) as u32 & 0x3F; // 6-bit stack pointer
                    if self.pos_stack_ptr > 30 {
                        self.gxstat |= 1 << 15; // GXSTAT bit 15: matrix stack error
                    }
                    let i = (self.pos_stack_ptr & 0x1F) as usize; // position stack index wraps at 32
                    self.pos_matrix = self.pos_stack[i];
                    self.vec_matrix = self.vec_stack[i];
                    self.clip_matrix_dirty = true;
                }
            },
            0x13 => match mode {
                // MTX_STORE - Store Current Matrix on Stack
                0 => self.proj_stack = self.proj_matrix,
                3 => self.tex_stack = self.tex_matrix,
                _ => {
                    let addr = (p0 & 0x1F) as usize; // stack index (bits 0-4)
                    if addr > 30 {
                        self.gxstat |= 1 << 15; // GXSTAT bit 15: matrix stack error
                    }
                    self.pos_stack[addr] = self.pos_matrix;
                    self.vec_stack[addr] = self.vec_matrix;
                }
            },
            0x14 => match mode {
                // MTX_RESTORE - Restore Current Matrix from Stack
                0 => {
                    self.proj_matrix = self.proj_stack;
                    self.clip_matrix_dirty = true;
                }
                3 => self.tex_matrix = self.tex_stack,
                _ => {
                    let addr = (p0 & 0x1F) as usize; // stack index (bits 0-4)
                    if addr > 30 {
                        self.gxstat |= 1 << 15; // GXSTAT bit 15: matrix stack error
                    }
                    self.pos_matrix = self.pos_stack[addr];
                    self.vec_matrix = self.vec_stack[addr];
                    self.clip_matrix_dirty = true;
                }
            },
            // DS 3D Matrix Load/Multiply: https://problemkaputt.de/gbatek.htm#ds3dmatrixloadmultiply
            0x15 => match mode {
                // MTX_IDENTITY - Load Unit Matrix to Current Matrix
                0 => {
                    self.proj_matrix = IDENTITY;
                    self.clip_matrix_dirty = true;
                }
                3 => self.tex_matrix = IDENTITY,
                _ => {
                    self.pos_matrix = IDENTITY;
                    if mode == 2 {
                        self.vec_matrix = IDENTITY;
                    }
                    self.clip_matrix_dirty = true;
                }
            },
            0x16..=0x1C => self.matrix_command(cmd, &params), // MTX_LOAD_4x4 .. MTX_TRANS
            // DS 3D Polygon Attributes: https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes
            0x20 => {
                // COLOR - Directly Set Vertex Color
                self.vertex_color = [
                    (p0 & 0x1F) as u8,         // red (bits 0-4)
                    ((p0 >> 5) & 0x1F) as u8,  // green (bits 5-9)
                    ((p0 >> 10) & 0x1F) as u8, // blue (bits 10-14)
                ];
            }
            // DS 3D Polygon Light Parameters: https://problemkaputt.de/gbatek.htm#ds3dpolygonlightparameters
            0x21 => {
                // NORMAL - Set Normal Vector
                self.normal = [sext10(p0, 0), sext10(p0, 10), sext10(p0, 20)];
                self.calculate_lighting();
            }
            // DS 3D Texture Coordinates: https://problemkaputt.de/gbatek.htm#ds3dtexturecoordinates
            0x22 => {
                // TEXCOORD - Set Texture Coordinates
                self.raw_texcoords = [(p0 & 0xFFFF) as i16, (p0 >> 16) as i16]; // S (bits 0-15), T (bits 16-31), 1.11.4 fixed point
                if self.tex_param >> 30 == 1 {
                    let (s, t) =
                        (i32::from(self.raw_texcoords[0]), i32::from(self.raw_texcoords[1]));
                    let m = &self.tex_matrix;
                    self.texcoords = core::array::from_fn(|i| {
                        ((s.wrapping_mul(m[i]))
                            .wrapping_add(t.wrapping_mul(m[4 + i]))
                            .wrapping_add(m[8 + i])
                            .wrapping_add(m[12 + i])
                            >> 12) as i16
                    });
                } else {
                    self.texcoords = self.raw_texcoords;
                }
            }
            // DS 3D Polygon Definitions by Vertices: https://problemkaputt.de/gbatek.htm#ds3dpolygondefinitionsbyvertices
            0x23 => {
                // VTX_16 - Set Vertex XYZ Coordinates (1.3.12 each)
                self.cur_vertex = [(p0 & 0xFFFF) as i16, (p0 >> 16) as i16, p[1] as i16]; // X (bits 0-15), Y (bits 16-31); Z is in the second parameter
                self.submit_vertex();
            }
            0x24 => {
                // VTX_10 - Set Vertex XYZ Coordinates (1.3.6 each)
                self.cur_vertex = [
                    ((p0 & 0x3FF) << 6) as i16,        // X (bits 0-9), shifted to 1.3.12
                    ((p0 & 0x000F_FC00) >> 4) as i16,  // Y (bits 10-19), shifted to 1.3.12
                    ((p0 & 0x3FF0_0000) >> 14) as i16, // Z (bits 20-29), shifted to 1.3.12
                ];
                self.submit_vertex();
            }
            0x25 => {
                // VTX_XY - Set Vertex XY Coordinates
                self.cur_vertex[0] = p0 as i16;
                self.cur_vertex[1] = (p0 >> 16) as i16;
                self.submit_vertex();
            }
            0x26 => {
                // VTX_XZ - Set Vertex XZ Coordinates
                self.cur_vertex[0] = p0 as i16;
                self.cur_vertex[2] = (p0 >> 16) as i16;
                self.submit_vertex();
            }
            0x27 => {
                // VTX_YZ - Set Vertex YZ Coordinates
                self.cur_vertex[1] = p0 as i16;
                self.cur_vertex[2] = (p0 >> 16) as i16;
                self.submit_vertex();
            }
            0x28 => {
                // VTX_DIFF - Set Relative Vertex Coordinates
                let v = &mut self.cur_vertex;
                v[0] = v[0].wrapping_add(sext10(p0, 0));
                v[1] = v[1].wrapping_add(sext10(p0, 10));
                v[2] = v[2].wrapping_add(sext10(p0, 20));
                self.submit_vertex();
            }
            // DS 3D Polygon Attributes: https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes
            0x29 => self.polygon_attr = p0, // POLYGON_ATTR - Set Polygon Attributes
            // DS 3D Texture Attributes: https://problemkaputt.de/gbatek.htm#ds3dtextureattributes
            0x2A => self.tex_param = p0, // TEXIMAGE_PARAM - Set Texture Parameters
            0x2B => self.tex_palette = p0 & 0x1FFF, // PLTT_BASE - palette base address (bits 0-12)
            // DS 3D Polygon Light Parameters: https://problemkaputt.de/gbatek.htm#ds3dpolygonlightparameters
            0x30 => {
                // DIF_AMB - MaterialColor0 - Diffuse/Ambient Reflect.
                self.mat_diffuse = [
                    (p0 & 0x1F) as u8,         // diffuse red (bits 0-4)
                    ((p0 >> 5) & 0x1F) as u8,  // diffuse green (bits 5-9)
                    ((p0 >> 10) & 0x1F) as u8, // diffuse blue (bits 10-14)
                ];
                self.mat_ambient = [
                    ((p0 >> 16) & 0x1F) as u8, // ambient red (bits 16-20)
                    ((p0 >> 21) & 0x1F) as u8, // ambient green (bits 21-25)
                    ((p0 >> 26) & 0x1F) as u8, // ambient blue (bits 26-30)
                ];
                if p0 & 0x8000 != 0 {
                    // bit 15: also set the diffuse color as vertex color
                    self.vertex_color = self.mat_diffuse;
                }
            }
            0x31 => {
                // SPE_EMI - MaterialColor1 - Specular Ref. & Emission
                self.mat_specular = [
                    (p0 & 0x1F) as u8,         // specular red (bits 0-4)
                    ((p0 >> 5) & 0x1F) as u8,  // specular green (bits 5-9)
                    ((p0 >> 10) & 0x1F) as u8, // specular blue (bits 10-14)
                ];
                self.mat_emission = [
                    ((p0 >> 16) & 0x1F) as u8, // emission red (bits 16-20)
                    ((p0 >> 21) & 0x1F) as u8, // emission green (bits 21-25)
                    ((p0 >> 26) & 0x1F) as u8, // emission blue (bits 26-30)
                ];
                self.use_shininess_table = p0 & 0x8000 != 0; // bit 15: use the specular reflection shininess table
            }
            0x32 => {
                // LIGHT_VECTOR - Set Light's Directional Vector
                let l = (p0 >> 30) as usize;
                let dir = [
                    i32::from(sext10(p0, 0)),
                    i32::from(sext10(p0, 10)),
                    i32::from(sext10(p0, 20)),
                ];
                let m = &self.vec_matrix;
                let dot = |c: usize| {
                    dir[0]
                        .wrapping_mul(m[c])
                        .wrapping_add(dir[1].wrapping_mul(m[4 + c]))
                        .wrapping_add(dir[2].wrapping_mul(m[8 + c]))
                };
                for c in 0..3 {
                    // DS 3D Polygon Light Parameters: https://problemkaputt.de/gbatek.htm#ds3dpolygonlightparameters
                    self.light_direction[l][c] = ((-(dot(c) >> 12)) << 21 >> 21) as i16; // light vector component, sign-extended 11 bits
                }
                let den = -((dot(2) << 9) >> 21) + (1 << 9); // 1.0 minus half-vector z, 9-bit fixed point
                self.spec_recip[l] = if den == 0 { 0 } else { (1 << 18) / den }; // reciprocal in 18-bit fixed point
            }
            0x33 => {
                // LIGHT_COLOR - Set Light Color
                let l = (p0 >> 30) as usize;
                self.light_color[l] = [
                    (p0 & 0x1F) as u8,         // red (bits 0-4)
                    ((p0 >> 5) & 0x1F) as u8,  // green (bits 5-9)
                    ((p0 >> 10) & 0x1F) as u8, // blue (bits 10-14)
                ];
            }
            0x34 => {
                // SHININESS - Specular Reflection Shininess Table
                for (chunk, word) in self.shininess_table.as_chunks_mut::<4>().0.iter_mut().zip(p) {
                    *chunk = word.to_le_bytes();
                }
            }
            // DS 3D Polygon Definitions by Vertices: https://problemkaputt.de/gbatek.htm#ds3dpolygondefinitionsbyvertices
            0x40 => {
                // BEGIN_VTXS - Start of Vertex List
                self.polygon_mode = p0 & 3;
                self.vertex_num_in_poly = 0;
                self.num_consecutive_polygons = 0;
                self.last_strip_polygon = None;
                self.cur_polygon_attr = self.polygon_attr;
            }
            0x41 => {} // END_VTXS - End of Vertex List (no effect on hardware)
            // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
            0x50 => {
                // SWAP_BUFFERS - Swap Rendering Engine Buffer
                self.swap_buffers = true;
                self.flush_attributes = p0 & 3;
            }
            0x60 => self.set_viewport(p0), // VIEWPORT - Set Viewport
            // DS 3D Tests: https://problemkaputt.de/gbatek.htm#ds3dtests
            0x70 => self.box_test(&p), // BOX_TEST - Test if Cuboid Sits inside View Volume
            0x71 => {
                // POS_TEST - Set Position Coordinates for Test
                self.cur_vertex = [(p0 & 0xFFFF) as i16, (p0 >> 16) as i16, p[1] as i16]; // X (bits 0-15), Y (bits 16-31); Z is in the second parameter
                self.update_clip_matrix();
                let v = self.cur_vertex.map(i32::from);
                self.pos_test_result = transform(v, &self.clip_matrix);
            }
            0x72 => {
                // VEC_TEST - Set Directional Vector for Test
                let n = [
                    i32::from(sext10(p0, 0)),
                    i32::from(sext10(p0, 10)),
                    i32::from(sext10(p0, 20)),
                ];
                let m = &self.vec_matrix;
                for c in 0..3 {
                    let mut r = (n[0]
                        .wrapping_mul(m[c])
                        .wrapping_add(n[1].wrapping_mul(m[4 + c]))
                        .wrapping_add(n[2].wrapping_mul(m[8 + c])))
                        >> 9;
                    if r & 0x1000 != 0 {
                        // bit 12 = sign of the 4.12 result
                        r |= 0xF000; // sign-extend into bits 12-15
                    }
                    self.vec_test_result[c] = r as i16;
                }
            }
            _ => {}
        }
    }
}
