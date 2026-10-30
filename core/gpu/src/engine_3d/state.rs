// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! 3D engine state: geometry engine (matrices, lighting, vertex/polygon
//! RAM) and the latched rendering state used by the rasterizer.
//!
//! The geometry engine follows melonDS `GPU3D.cpp` (GBATEK "DS 3D").

use std::collections::VecDeque;

/// Vertex RAM capacity per bank.
pub const MAX_VERTICES: usize = 6144;
/// Polygon RAM capacity per bank.
pub const MAX_POLYGONS: usize = 2048;

/// 4x4 fixed-point (20.12) matrix, row-major: `m[row * 4 + col]`.
pub type Mtx = [i32; 16];

#[rustfmt::skip]
pub const IDENTITY: Mtx = [
    // DS 3D Matrix Types: https://problemkaputt.de/gbatek.htm#ds3dmatrixtypes
    0x1000, 0, 0, 0, // 1.0 in 20.12 fixed point
    0, 0x1000, 0, 0, // 1.0 in 20.12 fixed point
    0, 0, 0x1000, 0, // 1.0 in 20.12 fixed point
    0, 0, 0, 0x1000, // 1.0 in 20.12 fixed point
];

/// Viewport (melonDS `Viewport[6]`): x0, y0 (bottom), x1, y1 (top),
/// width, height, in screen coordinates (y grows downwards).
#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone, Copy)]
pub struct ViewportReg {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub width: i32,
    pub height: i32,
}

/// One transformed vertex.
#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone, Copy)]
pub struct Vertex {
    /// Clip coordinates (x, y, z, w).
    pub position: [i32; 4],
    /// Color, 5-bit components in bits 12-16 (+0xFFF).
    pub color: [i32; 3],
    /// Color expanded to 9 bits for the rasterizer.
    pub final_color: [i32; 3],
    pub texcoords: [i16; 2],
    pub clipped: bool,
    /// Screen coordinates (x: 0-511, y: 0-255).
    pub final_position: [i32; 2],
    /// Screen coordinates in 1/16 pixels, for high-resolution rendering
    /// (melonDS `HiresPosition`).
    pub hires_position: [i32; 2],
}

/// One polygon in polygon RAM.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default)]
pub struct Polygon {
    /// Vertex RAM indices (in the polygon's bank).
    pub vertices: [usize; 10],
    pub num_vertices: usize,
    pub final_z: [i32; 10],
    pub final_w: [i32; 10],
    pub w_buffer: bool,

    pub attr: u32,
    pub tex_param: u32,
    pub tex_palette: u32,

    pub facing_view: bool,
    pub translucent: bool,
    pub is_shadow_mask: bool,
    pub is_shadow: bool,
    pub degenerate: bool,

    pub v_top: usize,
    pub v_bottom: usize,
    pub y_top: i32,
    pub y_bottom: i32,
    pub x_top: i32,
    pub x_bottom: i32,

    pub sort_key: u32,
}

/// GX command FIFO entry.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default)]
pub struct GxCommand {
    pub command: u8,
    pub param: u32,
}

/// Rendering registers latched at VBlank (melonDS `Render*`).
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RenderState {
    pub disp_cnt: u32,
    pub alpha_ref: u32,
    pub clear_attr1: u32,
    pub clear_attr2: u32,
    pub fog_color: u32,
    pub fog_offset: u32,
    pub fog_shift: u32,
    #[serde(with = "serde_big_array::BigArray")]
    pub fog_density: [u8; 34],
    pub edge_table: [u16; 8],
    pub toon_table: [u16; 32],
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            disp_cnt: 0,
            alpha_ref: 0,
            clear_attr1: 0,
            clear_attr2: 0,
            fog_color: 0,
            fog_offset: 0,
            fog_shift: 0,
            fog_density: [0; 34],
            edge_table: [0; 8],
            toon_table: [0; 32],
        }
    }
}

/// The DS 3D engine.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Gpu3D {
    /// Cycles the geometry engine is still busy for (run by the emulator).
    pub cycles: i64,

    // ===== GX FIFO / command decoding (driven by the emulator) =====
    pub gxfifo: VecDeque<GxCommand>,
    pub gxpipe: VecDeque<GxCommand>,
    /// Parameters of the command being executed.
    pub cmd_params: [u32; 32],
    pub cmd_param_count: u8,
    /// Packed-command decoding state of `GXFIFO` writes.
    pub param_count: u8,
    pub cmd_count: u8,
    pub total_params: u8,
    pub current_cmd: u32,

    // ===== registers =====
    /// DISP3DCNT (bits 12/13 are the underflow/overflow flags).
    pub disp_cnt: u32,
    pub alpha_ref_val: u32,
    pub clear_attr1: u32,
    pub clear_attr2: u32,
    pub fog_color: u32,
    pub fog_offset: u32,
    pub fog_density: [u8; 32],
    pub edge_table: [u16; 8],
    pub toon_table: [u16; 32],
    pub zero_dot_w_limit: i32,
    /// GXSTAT bits 1 (box test result), 15 (stack error), 30-31 (IRQ mode).
    pub gxstat: u32,

    // ===== geometry state =====
    pub matrix_mode: u32,
    pub proj_matrix: Mtx,
    pub pos_matrix: Mtx,
    pub vec_matrix: Mtx,
    pub tex_matrix: Mtx,
    pub clip_matrix: Mtx,
    pub clip_matrix_dirty: bool,

    pub proj_stack: Mtx,
    pub tex_stack: Mtx,
    pub pos_stack: [Mtx; 32],
    pub vec_stack: [Mtx; 32],
    pub proj_stack_ptr: u32,
    pub tex_stack_ptr: u32,
    pub pos_stack_ptr: u32,

    pub viewport: ViewportReg,

    pub polygon_attr: u32,
    pub cur_polygon_attr: u32,
    pub tex_param: u32,
    pub tex_palette: u32,

    pub polygon_mode: u32,
    pub cur_vertex: [i16; 3],
    pub vertex_color: [u8; 3],
    pub normal: [i16; 3],
    pub raw_texcoords: [i16; 2],
    pub texcoords: [i16; 2],

    pub light_direction: [[i16; 3]; 4],
    pub light_color: [[u8; 3]; 4],
    pub spec_recip: [i32; 4],
    pub mat_diffuse: [u8; 3],
    pub mat_ambient: [u8; 3],
    pub mat_specular: [u8; 3],
    pub mat_emission: [u8; 3],
    pub use_shininess_table: bool,
    #[serde(with = "serde_big_array::BigArray")]
    pub shininess_table: [u8; 128],

    pub pos_test_result: [i32; 4],
    pub vec_test_result: [i16; 3],

    /// Vertices of the polygon being assembled.
    pub temp_vertices: [Vertex; 4],
    pub vertex_num_in_poly: usize,
    pub num_consecutive_polygons: u32,
    /// Index (in the current bank) of the last strip polygon.
    pub last_strip_polygon: Option<usize>,

    // ===== vertex / polygon RAM (double buffered) =====
    pub vertex_ram: [Vec<Vertex>; 2],
    pub polygon_ram: [Vec<Polygon>; 2],
    pub cur_bank: usize,
    pub num_vertices: usize,
    pub num_polygons: usize,
    pub num_opaque_polygons: usize,

    /// SWAP_BUFFERS received; the geometry engine halts until VBlank.
    pub swap_buffers: bool,
    pub flush_attributes: u32,

    // ===== rendering =====
    /// Polygons to render (indices into `polygon_ram[render_bank]`), sorted.
    pub render_polygons: Vec<usize>,
    pub render_bank: usize,
    pub render: RenderState,
    /// The render list or rendering registers changed since the last
    /// rendered frame.
    pub render_dirty: bool,
    /// Number of polygons in the last swapped frame (debugging).
    pub rend_poly_count: i32,
    /// Number of vertices in the last swapped frame (debugging).
    pub rend_vert_count: i32,

    /// Rasterizer output and work buffers.
    #[serde(skip)]
    pub raster: crate::engine_3d::raster::Raster,
    /// Display-only high-resolution copy of the 3D frame.
    #[serde(skip)]
    pub hires: crate::engine_3d::raster::hires::Hires,
}
