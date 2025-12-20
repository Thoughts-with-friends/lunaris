// CorgiDS Copyright PSISP 2017
// Licensed under the GPLv3
// See LICENSE.txt for details
mod consts;

use mem_const::*;
use std::collections::VecDeque;

/// GPU 3D Display Control Register
#[derive(Default)]
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

/// Texture Image Parameter Register
#[derive(Default, Clone)]
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

/// Polygon Attribute Register
#[derive(Default, Clone)]
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

/// Viewport Register
#[derive(Default)]
pub struct ViewportReg {
    pub x1: u8,
    pub y1: u8,
    pub x2: u8,
    pub y2: u8,
}

/// GX Status Register
#[derive(Default)]
pub struct GxStatReg {
    pub box_pos_vec_busy: bool,
    pub boxtest_result: bool,
    pub mtx_stack_busy: bool,
    pub mtx_overflow: bool,
    pub geo_busy: bool,
    pub gfxifo_irq_stat: i32,
}

/// 4x4 Matrix
/// - C++ `Mtx`
#[derive(Clone, Default)]
pub struct Matrix {
    pub m: [[i32; 4]; 4],
}

impl Matrix {
    #[inline]
    pub const fn new(m: [[i32; 4]; 4]) -> Self {
        Self { m }
    }

    /// Set this matrix to another
    pub fn set(&mut self, other: &Matrix) {
        todo!()
    }
}

/// Vertex structure
#[derive(Default, Clone)]
pub struct Vertex {
    pub coords: [i32; 4],
    pub colors: [i32; 3],
    pub final_colors: [i32; 3],
    pub clipped: bool,
    pub texcoords: [i32; 2],
}

/// Polygon structure
#[derive(Default, Clone)]
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
#[derive(Default)]
pub struct GxCommand {
    pub command: u8,
    pub param: u32,
}

/// 3D GPU emulator
pub struct Gpu3D {
    /// Remaining cycles
    cycles: i32,

    /// 3D display control
    disp3dcnt: Disp3DCntReg,

    /// Current polygon attributes
    polygon_attr: PolygonAttrReg,

    /// Texture parameters
    teximage_param: TexImageParamReg,

    /// Toon shading table (32 entries)
    toon_table: [u16; 32],

    /// Texture palette base address
    pltt_base: u32,

    /// Viewport settings
    viewport: ViewportReg,

    /// Geometry engine status
    gxstat: GxStatReg,

    /// Polygon primitive type
    polygon_type: u32,

    /// Z-buffer clear value
    clear_depth: u32,

    /// Color buffer clear value
    clear_color: u32,

    /// FIFO flush mode
    flush_mode: i32,

    /// Command FIFO
    gfxifo: VecDeque<GxCommand>,

    /// Command pipeline
    gxpipe: VecDeque<GxCommand>,

    /// Command parameters
    /// - `[u32; 32]`
    cmd_params: Vec<u32>,

    /// Received parameter count
    param_count: u8,

    /// Expected parameter count
    cmd_param_count: u8,

    /// Processed command count
    cmd_count: u8,

    /// Total parameters
    total_params: u8,

    /// Current command opcode
    current_cmd: u32,

    /// Latched polygon attributes
    current_poly_attr: PolygonAttrReg,

    /// Current vertex color
    current_color: u32,

    /// Current vertex position
    current_vertex: [i16; 3],

    /// Current texture coordinates
    current_texcoords: [i16; 2],

    /// Depth buffer
    /// - `[[u32; PIXELS_PER_LINE]; SCANLINES]`
    z_buffer: Vec<Vec<u32>>,

    /// Transparent polygon IDs
    /// - `[u8; PIXELS_PER_LINE]`
    trans_poly_ids: Vec<u8>,

    /// Swap geometry/render buffers
    swap_buffers: bool,

    /// Geometry vertices
    /// - `[Vertex; 6188]`
    geo_vert: Vec<Vertex>,

    /// Render vertices
    /// - `[Vertex; 6188]`
    rend_vert: Vec<Vertex>,

    /// Geometry polygons
    /// - `[Polygon; 2048]`
    geo_poly: Vec<Polygon>,

    /// Render polygons
    /// - `[Polygon; 2048]`
    rend_poly: Vec<Polygon>,

    /// Last strip polygon position
    /// - `Option<*mut Polygon>`
    last_poly_strip: Option<usize>,

    /// Temporary vertex list
    /// - `[Vertex; 10]`
    vertex_list: Vec<Vertex>,

    /// Vertex list size
    vertex_list_count: i32,

    /// Geometry vertex count
    geo_vert_count: i32,

    /// Render vertex count
    rend_vert_count: i32,

    /// Geometry polygon count
    geo_poly_count: i32,

    /// Render polygon count
    rend_poly_count: i32,

    /// Consecutive polygon count
    consecutive_polygons: i32,

    /// 16-bit vertex index
    vtx_16_index: i32,

    /// Current matrix mode
    mtx_mode: u8,

    /// Projection matrix
    projection_mtx: Matrix,

    /// Vector matrix
    vector_mtx: Matrix,

    /// Model-view matrix
    modelview_mtx: Matrix,

    /// Texture matrix
    texture_mtx: Matrix,

    /// Projection stack
    projection_stack: Matrix,

    /// Texture stack
    texture_stack: Matrix,

    /// Model-view stack
    /// - `[Matrix; 0x20]`
    modelview_stack: Vec<Matrix>,

    /// Vector stack
    /// - `[Matrix; 0x20]`
    vector_stack: Vec<Matrix>,

    /// Clip matrix
    clip_mtx: Matrix,

    /// Clip matrix dirty flag
    clip_dirty: bool,

    /// Model-view stack pointer
    modelview_sp: u8,

    /// Emission color
    emission_color: u16,

    /// Ambient color
    ambient_color: u16,

    /// Diffuse color
    diffuse_color: u16,

    /// Specular color
    specular_color: u16,

    /// Light colors
    light_color: [u16; 4],

    /// Light directions
    light_direction: [[i16; 3]; 4],

    /// Normal vector
    normal_vector: [i16; 3],

    /// Shininess table
    /// - [u8; 128]
    shine_table: Vec<u8>,

    /// Use shininess table
    using_shine_table: bool,

    /// Vector test result
    vec_test_result: [i16; 3],

    /// Matrix multiply parameters
    mult_params: Matrix,

    /// C++ int
    /// Matrix parameter index
    mult_params_index: usize,
}

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
            modelview_sp: 0,
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

    pub fn render_scanline(
        &mut self,
        framebuffer: &mut [u32],
        bg_priorities: &[u8],
        bg0_priority: u8,
    ) {
        todo!()
    }

    pub fn run(&mut self, cycles_to_run: u64) {
        todo!()
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
