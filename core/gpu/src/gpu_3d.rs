/*
    CorgiDS Copyright PSISP 2017
    Licensed under the GPLv3
    See LICENSE.txt for details
*/

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
#[derive(Default)]
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
#[derive(Default)]
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
#[derive(Clone, Default)]
pub struct Mtx {
    pub m: [[i32; 4]; 4],
}

impl Mtx {
    /// Set this matrix to another
    pub fn set(&mut self, other: &Mtx) {
        todo!()
    }
}

/// Vertex structure
#[derive(Default)]
pub struct Vertex {
    pub coords: [i32; 4],
    pub colors: [i32; 3],
    pub final_colors: [i32; 3],
    pub clipped: bool,
    pub texcoords: [i32; 2],
}

/// Polygon structure
#[derive(Default)]
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

pub struct Emulator;
pub struct Gpu;

/// 3D GPU emulator
pub struct Gpu3D {
    e: *mut Emulator,
    gpu: *mut Gpu,
    cycles: i32,
    disp3dcnt: Disp3DCntReg,
    polygon_attr: PolygonAttrReg,
    teximage_param: TexImageParamReg,
    toon_table: [u16; 32],
    pltt_base: u32,
    viewport: ViewportReg,
    gxstat: GxStatReg,
    polygon_type: u32,
    clear_depth: u32,
    clear_color: u32,
    flush_mode: i32,
    gfxifo: VecDeque<GxCommand>,
    gxpipe: VecDeque<GxCommand>,
    cmd_params: [u32; 32],
    param_count: u8,
    cmd_param_count: u8,
    cmd_count: u8,
    total_params: u8,
    current_cmd: u32,
    current_poly_attr: PolygonAttrReg,
    current_color: u32,
    current_vertex: [i16; 3],
    current_texcoords: [i16; 2],
    z_buffer: [[u32; PIXELS_PER_LINE]; SCANLINES],
    trans_poly_ids: [u8; PIXELS_PER_LINE],
    swap_buffers: bool,
    geo_vert: [Vertex; 6188],
    rend_vert: [Vertex; 6188],
    geo_poly: [Polygon; 2048],
    rend_poly: [Polygon; 2048],
    last_poly_strip: Option<*mut Polygon>,
    vertex_list: [Vertex; 10],
    vertex_list_count: i32,
    geo_vert_count: i32,
    rend_vert_count: i32,
    geo_poly_count: i32,
    rend_poly_count: i32,
    consecutive_polygons: i32,
    vtx_16_index: i32,
    mtx_mode: u8,
    projection_mtx: Mtx,
    vector_mtx: Mtx,
    modelview_mtx: Mtx,
    texture_mtx: Mtx,
    projection_stack: Mtx,
    texture_stack: Mtx,
    modelview_stack: [Mtx; 0x20],
    vector_stack: [Mtx; 0x20],
    clip_mtx: Mtx,
    clip_dirty: bool,
    modelview_sp: u8,
    emission_color: u16,
    ambient_color: u16,
    diffuse_color: u16,
    specular_color: u16,
    light_color: [u16; 4],
    light_direction: [[i16; 3]; 4],
    normal_vector: [i16; 3],
    shine_table: [u8; 128],
    using_shine_table: bool,
    vec_test_result: [i16; 3],
}

impl Gpu3D {
    pub fn new(e: *mut Emulator, gpu: *mut Gpu) -> Self {
        todo!()
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

// Constants to define
const SCANLINES: usize = 256;
const PIXELS_PER_LINE: usize = 256;
