// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp
//!

use std::collections::VecDeque;

/// GPU 3D Display Control Register
#[derive(Debug, Default)]
pub struct Disp3DCntReg {
    pub texture_mapping: bool,
    pub highlight_shading: bool,
    pub alpha_test: bool,
    pub alpha_blending: bool,
    pub anti_aliasing: bool,
    pub edge_marking: bool,
    pub fog_color_mode: bool,
    pub fog_enable: bool,
    pub fog_depth_shift: i32,
    pub color_buffer_underflow: bool,
    pub ram_overflow: bool,
    pub rear_plane_mode: bool,
}

impl Disp3DCntReg {
    pub fn get(&self) -> u16 {
        let mut reg: u16 = 0;

        reg |= self.texture_mapping as u16;
        reg |= (self.highlight_shading as u16) << 1;
        reg |= (self.alpha_test as u16) << 2;
        reg |= (self.alpha_blending as u16) << 3;
        reg |= (self.anti_aliasing as u16) << 4;
        reg |= (self.edge_marking as u16) << 5;
        reg |= (self.fog_color_mode as u16) << 6;
        reg |= (self.fog_enable as u16) << 7;
        reg |= (self.fog_depth_shift as u16) << 8;
        reg |= (self.color_buffer_underflow as u16) << 12;
        reg |= (self.ram_overflow as u16) << 13;
        reg |= (self.rear_plane_mode as u16) << 14;

        reg
    }
}

/// Texture Image Parameter Register
#[derive(Debug, Default, Clone)]
pub struct TexImageParamReg {
    pub vram_offset: i32,
    pub repeat_s: bool,
    pub repeat_t: bool,
    pub flip_s: bool,
    pub flip_t: bool,
    pub s_size: i32,
    pub t_size: i32,
    pub format: i32,
    pub color0_transparent: bool,
    pub transformation_mode: i32,
}

impl TexImageParamReg {
    pub(crate) fn set(&mut self, word: u32) {
        #[cfg(feature = "tracing")]
        tracing::debug!("Set TEXIMAGE_PARAM: {word:08X}");

        self.vram_offset = (word & 0xFFFF) as i32;
        self.repeat_s = (word & (1 << 16)) != 0;
        self.repeat_t = (word & (1 << 17)) != 0;
        self.flip_s = (word & (1 << 18)) != 0;
        self.flip_t = (word & (1 << 19)) != 0;
        self.s_size = ((word >> 20) & 0x7) as i32;
        self.t_size = ((word >> 23) & 0x7) as i32;
        self.format = ((word >> 26) & 0x7) as i32;
        self.color0_transparent = (word & (1 << 29)) != 0;
        self.transformation_mode = (word >> 30) as i32;
    }
}

/// Polygon Attribute Register
#[derive(Debug, Default, Clone)]
pub struct PolygonAttrReg {
    pub light_enable: i32,
    pub polygon_mode: i32,
    pub render_back: bool,
    pub render_front: bool,
    pub set_new_trans_depth: bool,
    pub render_1dot: bool,
    pub render_far_intersect: bool,
    pub depth_test_equal: bool,
    pub fog_enable: bool,
    pub alpha: i32,
    pub id: i32,
}

impl PolygonAttrReg {
    pub(crate) fn set(&mut self, word: u32) {
        self.light_enable = (word & 0xF) as i32;
        self.polygon_mode = ((word >> 4) & 0x3) as i32;
        self.render_back = (word & (1 << 6)) != 0;
        self.render_front = (word & (1 << 7)) != 0;
        self.set_new_trans_depth = (word & (1 << 11)) != 0;
        self.render_far_intersect = (word & (1 << 12)) != 0;
        self.render_1dot = (word & (1 << 13)) != 0;
        self.depth_test_equal = (word & (1 << 14)) != 0;
        self.fog_enable = (word & (1 << 15)) != 0;
        self.alpha = ((word >> 16) & 0x1F) as i32;
        self.id = ((word >> 24) & 0x3F) as i32;
    }
}

/// Viewport Register
#[derive(Debug, Default)]
pub struct ViewportReg {
    pub x1: u8,
    pub y1: u8,
    pub x2: u8,
    pub y2: u8,
}

/// GX Status Register
#[derive(Debug, Default)]
pub struct GxStatReg {
    pub box_pos_vec_busy: bool,
    pub boxtest_result: bool,
    pub mtx_stack_busy: bool,
    pub mtx_overflow: bool,
    pub geo_busy: bool,
    pub gxfifo_irq_stat: i32,
}

/// 4x4 Matrix
/// - C++ `Mtx`
#[derive(Debug, Clone, Default)]
pub struct Matrix {
    pub m: [[i32; 4]; 4],
}

impl Matrix {
    #[inline]
    pub const fn new(m: [[i32; 4]; 4]) -> Self {
        Self { m }
    }

    #[inline]
    pub const fn zeros() -> Self {
        let m = [[0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]];
        Self { m }
    }

    /// Set this matrix to another
    pub fn set(&mut self, other: &Matrix) {
        self.m = other.m;
    }

    /// Set as identity
    pub fn set_identity(&mut self) {
        self.m = super::consts::IDENTITY_MATRIX.clone().m;
    }
}

/// Vertex structure
#[derive(Debug, Default, Clone)]
pub struct Vertex {
    pub coords: [i32; 4],
    pub colors: [i32; 3],
    pub final_colors: [i32; 3],
    pub clipped: bool,
    pub texcoords: [i32; 2],
}

/// Polygon structure
#[derive(Debug, Default, Clone)]
pub struct Polygon {
    pub vert_index: u16,
    pub vertices: u8,
    pub top_y: u16,
    pub bottom_y: u16,
    pub attributes: PolygonAttrReg,
    pub texparams: TexImageParamReg,
    pub palette_base: u32,
    pub translucent: bool,
}

/// GX Command
#[derive(Debug, Clone, Default)]
pub struct GxCommand {
    pub command: u8,
    pub param: u32,
}

/// 3D GPU emulator
#[derive(Debug, Default)]
pub struct Gpu3D {
    /// Remaining cycles
    pub cycles: i64,

    /// 3D display control
    pub disp3dcnt: Disp3DCntReg,
    /// Current polygon attributes
    pub polygon_attr: PolygonAttrReg,
    /// Texture parameters
    pub teximage_param: TexImageParamReg,
    /// Toon shading table (32 entries)
    pub toon_table: [u16; 32],
    /// Texture palette base address
    pub pltt_base: u32,
    /// Viewport settings
    pub viewport: ViewportReg,
    /// Geometry engine status
    pub gxstat: GxStatReg,
    /// Polygon primitive type
    pub polygon_type: u32,
    /// Z-buffer clear value
    pub clear_depth: u32,
    /// Color buffer clear value
    pub clear_color: u32,
    /// FIFO flush mode
    pub flush_mode: i32,

    /// Command FIFO
    pub gxfifo: VecDeque<GxCommand>,
    /// Command pipeline
    pub gxpipe: VecDeque<GxCommand>,

    /// Command parameters
    /// - `[u32; 32]`
    pub cmd_params: Vec<u32>,
    /// Received parameter count
    pub param_count: u8,
    /// Expected parameter count
    pub cmd_param_count: u8,
    /// Processed command count
    pub cmd_count: u8,
    /// Total parameters
    pub total_params: u8,
    /// Current command opcode
    pub current_cmd: u32,
    /// Latched polygon attributes
    pub current_poly_attr: PolygonAttrReg,

    /// Current vertex color
    pub current_color: u32,
    /// Current vertex position
    pub current_vertex: [i16; 3],
    /// Current texture coordinates
    pub current_texcoords: [i16; 2],

    /// Depth buffer
    /// - `[[u32; PIXELS_PER_LINE]; SCANLINES]`
    pub z_buffer: Vec<Vec<u32>>,
    /// Transparent polygon IDs
    /// - `[u8; PIXELS_PER_LINE]`
    pub trans_poly_ids: Vec<u8>,

    /// Swap geometry/render buffers
    pub swap_buffers: bool,

    /// Geometry vertices
    /// - `[Vertex; 6188]`
    pub geo_vert: Vec<Vertex>,

    /// Render vertices
    /// - `[Vertex; 6188]`
    pub rend_vert: Vec<Vertex>,
    /// Geometry polygons
    /// - `[Polygon; 2048]`
    pub geo_poly: Vec<Polygon>,
    /// Render polygons
    /// - `[Polygon; 2048]`
    pub rend_poly: Vec<Polygon>,
    /// Last strip polygon position (self.geo_poly index)
    ///
    /// - C++ type `*mut Polygon`
    pub last_poly_strip: Option<usize>,

    /// Temporary vertex list
    /// - `[Vertex; 10]`
    pub vertex_list: Vec<Vertex>,
    /// Vertex list size
    pub vertex_list_count: i32,

    /// Geometry vertex count
    pub geo_vert_count: i32,
    /// Render vertex count
    pub rend_vert_count: i32,

    /// Geometry polygon count
    pub geo_poly_count: i32,
    /// Render polygon count
    pub rend_poly_count: i32,

    /// Consecutive polygon count
    pub consecutive_polygons: i32,

    /// 16-bit vertex index
    pub vtx_16_index: i32,

    /// Current matrix mode
    pub mtx_mode: u8,

    /// Projection matrix
    pub projection_mtx: Matrix,
    /// Vector matrix
    pub vector_mtx: Matrix,
    /// Model-view matrix
    pub modelview_mtx: Matrix,
    /// Texture matrix
    pub texture_mtx: Matrix,

    /// Projection stack
    pub projection_stack: Matrix,
    /// Texture stack
    pub texture_stack: Matrix,
    /// Model-view stack
    /// - `[Matrix; 0x20]`
    pub modelview_stack: Vec<Matrix>,
    /// Vector stack
    /// - `[Matrix; 0x20]`
    pub vector_stack: Vec<Matrix>,

    /// Clip matrix
    pub clip_mtx: Matrix,
    /// Clip matrix dirty flag
    pub clip_dirty: bool,
    /// Model-view stack pointer
    pub model_view_sp: u8,

    /// Emission color
    pub emission_color: u16,
    /// Ambient color
    pub ambient_color: u16,
    /// Diffuse color
    pub diffuse_color: u16,
    /// Specular color
    pub specular_color: u16,

    /// Light colors
    pub light_color: [u16; 4],
    /// Light directions
    pub light_direction: [[i16; 3]; 4],

    /// Normal vector
    pub normal_vector: [i16; 3],
    /// Shininess table
    /// - [u8; 128]
    pub shine_table: Vec<u8>,
    /// Use shininess table
    pub using_shine_table: bool,

    /// Vector test result
    pub vec_test_result: [i16; 3],

    /// Matrix multiply parameters
    pub mult_params: Matrix,
    /// C++ int
    /// Matrix parameter index
    pub mult_params_index: usize,
}
