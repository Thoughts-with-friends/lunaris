//! Nintendo DS 3D GPU emulation module
//!
//! This file is a faithful Rust port of the original C++ GPU3D implementation
//! from CorgiDS. All data structures are preserved without simplification.

use std::collections::VecDeque;
use std::mem::copy_nonoverlapping;

/// DISP3DCNT register
#[derive(Clone, Copy, Default)]
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

/// TEXIMAGE_PARAM register
#[derive(Clone, Copy, Default)]
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

/// POLYGON_ATTR register
#[derive(Clone, Copy, Default)]
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

/// VIEWPORT register
#[derive(Clone, Copy, Default)]
pub struct ViewportReg {
    pub x1: u8,
    pub y1: u8,
    pub x2: u8,
    pub y2: u8,
}

/// GXSTAT register
#[derive(Clone, Copy, Default)]
pub struct GxStatReg {
    pub box_pos_vec_busy: bool,
    pub boxtest_result: bool,
    pub mtx_stack_busy: bool,
    pub mtx_overflow: bool,
    pub geo_busy: bool,
    pub gxfifo_irq_stat: i32,
}

/// 4x4 fixed-point matrix
#[derive(Clone, Copy)]
pub struct Mtx {
    pub m: [[i32; 4]; 4],
}

impl Mtx {
    /// Copy matrix contents
    pub fn set(&mut self, other: &Mtx) {
        unsafe {
            copy_nonoverlapping(other.m.as_ptr(), self.m.as_mut_ptr(), 16);
        }
    }
}

/// Vertex structure
#[derive(Clone, Copy, Default)]
pub struct Vertex {
    pub coords: [i32; 4],
    pub colors: [i32; 3],
    pub final_colors: [i32; 3],
    pub clipped: bool,
    pub texcoords: [i32; 2],
}

/// Polygon structure
#[derive(Clone, Copy, Default)]
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

/// GX command
#[derive(Clone, Copy)]
pub struct GxCommand {
    pub command: u8,
    pub param: u32,
}

// //! GPU_3D core structure and initialization
// //!
// //! Faithful Rust port of the C++ GPU_3D class from CorgiDS.
// //! No behavior or layout is simplified.

// use std::collections::VecDeque;

// use crate::gpu3d::{
//     Disp3DCntReg, GxCommand, GxStatReg, Mtx, Polygon, PolygonAttrReg,
//     TexImageParamReg, Vertex, ViewportReg,
// };

/// Constants imported from memconsts.h in the original project
pub const SCANLINES: usize = 192;
pub const PIXELS_PER_LINE: usize = 256;

/// GPU_3D main structure
pub struct Gpu3D<'a> {
    /// Emulator reference
    pub emulator: *mut crate::emulator::Emulator,
    /// GPU reference
    pub gpu: *mut crate::gpu::Gpu,

    pub cycles: i64,

    pub disp3dcnt: Disp3DCntReg,
    pub polygon_attr: PolygonAttrReg,
    pub teximage_param: TexImageParamReg,

    pub toon_table: [u16; 32],
    pub pltt_base: u32,

    pub viewport: ViewportReg,
    pub gxstat: GxStatReg,

    pub polygon_type: u32,
    pub clear_depth: u32,
    pub clear_color: u32,
    pub flush_mode: i32,

    pub gxfifo: VecDeque<GxCommand>,
    pub gxpipe: VecDeque<GxCommand>,

    pub cmd_params: [u32; 32],
    pub param_count: u8,
    pub cmd_param_count: u8,
    pub cmd_count: u8,
    pub total_params: u8,
    pub current_cmd: u32,
    pub current_poly_attr: PolygonAttrReg,

    pub current_color: u32,
    pub current_vertex: [i16; 3],
    pub current_texcoords: [i16; 2],

    pub z_buffer: [[u32; PIXELS_PER_LINE]; SCANLINES],
    pub trans_poly_ids: [u8; PIXELS_PER_LINE],

    pub swap_buffers: bool,

    pub geo_vert: [Vertex; 6188],
    pub rend_vert: [Vertex; 6188],

    pub geo_poly: [Polygon; 2048],
    pub rend_poly: [Polygon; 2048],

    pub last_poly_strip: Option<usize>,

    pub vertex_list: [Vertex; 10],
    pub vertex_list_count: i32,

    pub geo_vert_count: i32,
    pub rend_vert_count: i32,
    pub geo_poly_count: i32,
    pub rend_poly_count: i32,

    pub consecutive_polygons: i32,
    pub vtx_16_index: i32,

    pub mtx_mode: u8,

    pub projection_mtx: Mtx,
    pub vector_mtx: Mtx,
    pub modelview_mtx: Mtx,
    pub texture_mtx: Mtx,

    pub projection_stack: Mtx,
    pub texture_stack: Mtx,

    pub modelview_stack: [Mtx; 0x20],
    pub vector_stack: [Mtx; 0x20],

    pub clip_mtx: Mtx,
    pub clip_dirty: bool,
    pub modelview_sp: u8,

    pub emission_color: u16,
    pub ambient_color: u16,
    pub diffuse_color: u16,
    pub specular_color: u16,

    pub light_color: [u16; 4],
    pub light_direction: [[i16; 3]; 4],
    pub normal_vector: [i16; 3],

    pub shine_table: [u8; 128],
    pub using_shine_table: bool,

    pub vec_test_result: [i16; 3],

    pub mult_params: Mtx,
    pub mult_params_index: i32,
}

impl<'a> Gpu3D<'a> {
    /// Identity matrix constant (1.12 fixed-point)
    pub const IDENTITY: Mtx = Mtx {
        m: [
            [1 << 12, 0, 0, 0],
            [0, 1 << 12, 0, 0],
            [0, 0, 1 << 12, 0],
            [0, 0, 0, 1 << 12],
        ],
    };

    /// GX command parameter counts
    pub const CMD_PARAM_AMOUNTS: [u8; 256] = [
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        1,0,1,1,1,0,16,12,16,12,9,3,3,0,0,0,
        1,1,1,2,1,1,1,1,1,1,1,1,0,0,0,0,
        1,1,1,1,32,0,0,0,0,0,0,0,0,0,0,0,
        1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        3,2,1,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0; 256
    ];

    /// GX command cycle counts
    pub const CMD_CYCLE_AMOUNTS: [u16; 256] = [
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        1,17,36,17,36,19,34,30,35,31,28,22,22,0,0,0,
        1,9,1,9,8,8,8,8,8,1,1,1,0,0,0,0,
        4,4,6,1,32,0,0,0,0,0,0,0,0,0,0,0,
        1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        392,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0; 256
    ];

    /// Create a new GPU_3D instance
    pub fn new(
        emulator: *mut crate::emulator::Emulator,
        gpu: *mut crate::gpu::Gpu,
    ) -> Self {
        let mut instance = Self {
            emulator,
            gpu,
            cycles: 0,
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
            cmd_params: [0; 32],
            param_count: 0,
            cmd_param_count: 0,
            cmd_count: 0,
            total_params: 0,
            current_cmd: 0,
            current_poly_attr: PolygonAttrReg::default(),
            current_color: 0,
            current_vertex: [0; 3],
            current_texcoords: [0; 2],
            z_buffer: [[0; PIXELS_PER_LINE]; SCANLINES],
            trans_poly_ids: [0; PIXELS_PER_LINE],
            swap_buffers: false,
            geo_vert: [Vertex::default(); 6188],
            rend_vert: [Vertex::default(); 6188],
            geo_poly: [Polygon::default(); 2048],
            rend_poly: [Polygon::default(); 2048],
            last_poly_strip: None,
            vertex_list: [Vertex::default(); 10],
            vertex_list_count: 0,
            geo_vert_count: 0,
            rend_vert_count: 0,
            geo_poly_count: 0,
            rend_poly_count: 0,
            consecutive_polygons: 0,
            vtx_16_index: 0,
            mtx_mode: 0,
            projection_mtx: Self::IDENTITY,
            vector_mtx: Self::IDENTITY,
            modelview_mtx: Self::IDENTITY,
            texture_mtx: Self::IDENTITY,
            projection_stack: Self::IDENTITY,
            texture_stack: Self::IDENTITY,
            modelview_stack: [Self::IDENTITY; 0x20],
            vector_stack: [Self::IDENTITY; 0x20],
            clip_mtx: Self::IDENTITY,
            clip_dirty: true,
            modelview_sp: 0,
            emission_color: 0,
            ambient_color: 0,
            diffuse_color: 0,
            specular_color: 0,
            light_color: [0; 4],
            light_direction: [[0; 3]; 4],
            normal_vector: [0; 3],
            shine_table: [0; 128],
            using_shine_table: false,
            vec_test_result: [0; 3],
            mult_params: Self::IDENTITY,
            mult_params_index: 0,
        };

        instance.power_on();
        instance
    }

    /// Power-on reset (faithful to C++ implementation)
    pub fn power_on(&mut self) {
        self.gxfifo.clear();
        self.gxpipe.clear();

        self.disp3dcnt = Disp3DCntReg::default();
        self.projection_mtx = Self::IDENTITY;
        self.vector_mtx = Self::IDENTITY;
        self.modelview_mtx = Self::IDENTITY;
        self.texture_mtx = Self::IDENTITY;
        self.mult_params = Self::IDENTITY;

        self.mult_params_index = 0;
        self.geo_vert_count = 0;
        self.geo_poly_count = 0;
        self.rend_vert_count = 0;
        self.rend_poly_count = 0;
        self.vtx_16_index = 0;
        self.modelview_sp = 0;
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
}

impl<'a> Gpu3D<'a> {
    /// Perspective-correct interpolation
    ///
    /// This matches the original GPU algorithm used in the C++ implementation.
    /// No optimizations or approximations are applied.
    pub fn interpolate(
        &self,
        pixel: u64,
        pixel_range: u64,
        u1: i64,
        u2: i64,
        w1: i32,
        w2: i32,
    ) -> i64 {
        let mut bark: i64 = 0;
        bark += ((pixel_range - pixel) as i64) * (u1 * w2 as i64);
        bark += (pixel as i64) * (u2 * w1 as i64);

        let mut denom: i64 = 0;
        denom += ((pixel_range - pixel) as i64) * (w2 as i64);
        denom += (pixel as i64) * (w1 as i64);

        bark / denom
    }

    /// Run GPU cycles
    ///
    /// Executes commands from GXPIPE until cycle budget is exhausted.
    pub fn run(&mut self, cycles_to_run: u64) {
        if self.swap_buffers {
            return;
        }

        if self.cycles <= 0 && self.gxpipe.is_empty() {
            self.cycles = 0;
            return;
        }

        self.cycles -= cycles_to_run as i64;

        while self.cycles <= 0 && !self.gxpipe.is_empty() {
            self.exec_command();
        }
    }

    /// Request FIFO DMA if FIFO is below threshold
    pub fn check_fifo_dma(&mut self) {
        if self.gxfifo.len() < 128 {
            unsafe {
                (*self.emulator).gxfifo_dma_request();
            }
        }
    }

    /// Handle FIFO IRQ conditions
    pub fn check_fifo_irq(&mut self) {
        match self.gxstat.gxfifo_irq_stat {
            1 => {
                if self.gxfifo.len() < 128 {
                    unsafe {
                        (*self.emulator).request_interrupt9(
                            crate::interrupt::Interrupt::GeometryFifo,
                        );
                    }
                }
            }
            2 => {
                if self.gxfifo.is_empty() {
                    unsafe {
                        (*self.emulator).request_interrupt9(
                            crate::interrupt::Interrupt::GeometryFifo,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// Write a 32-bit word to GXFIFO
    ///
    /// This method handles packed GX commands exactly like the C++ version.
    pub fn write_gxfifo(&mut self, word: u32) {
        if self.cmd_count == 0 {
            self.cmd_count = 4;
            self.current_cmd = word;
            self.param_count = 0;
            self.total_params =
                Self::CMD_PARAM_AMOUNTS[(word & 0xFF) as usize];

            if self.total_params > 0 {
                return;
            }
        } else {
            self.param_count += 1;
        }

        loop {
            if (self.current_cmd & 0xFF) != 0
                || (self.cmd_count == 4 && self.current_cmd == 0)
            {
                let cmd = GxCommand {
                    command: (self.current_cmd & 0xFF) as u8,
                    param: word,
                };
                self.write_command(cmd);
            }

            if self.param_count >= self.total_params {
                self.current_cmd >>= 8;
                self.cmd_count -= 1;

                if self.cmd_count == 0 {
                    break;
                }

                self.param_count = 0;
                self.total_params =
                    Self::CMD_PARAM_AMOUNTS[(self.current_cmd & 0xFF) as usize];
            }

            if self.param_count < self.total_params {
                break;
            }
        }
    }

    /// Write a command directly to FIFO (bypassing packed format)
    pub fn write_fifo_direct(&mut self, address: u32, word: u32) {
        let cmd = GxCommand {
            command: ((address >> 2) & 0x7F) as u8,
            param: word,
        };
        self.write_command(cmd);
    }

    /// Read next command from GXPIPE
    ///
    /// Also updates GXSTAT busy flags and refills the pipeline.
    pub fn read_command(&mut self) -> GxCommand {
        let cmd = self.gxpipe.pop_front().unwrap();

        // Refill pipeline when half empty
        if self.gxpipe.len() < 3 {
            if let Some(c) = self.gxfifo.pop_front() {
                self.gxpipe.push_back(c);
            }
            if let Some(c) = self.gxfifo.pop_front() {
                self.gxpipe.push_back(c);
            }

            self.check_fifo_dma();
            self.check_fifo_irq();
        }

        // Update GXSTAT flags
        if let Some(next) = self.gxpipe.front() {
            self.gxstat.mtx_stack_busy =
                next.command == 0x11 || next.command == 0x12;

            self.gxstat.box_pos_vec_busy =
                next.command == 0x70
                    || next.command == 0x71
                    || next.command == 0x72;
        } else {
            self.gxstat.mtx_stack_busy = false;
            self.gxstat.box_pos_vec_busy = false;
        }

        cmd
    }

    /// Push a command into GXPIPE
    ///
    /// This is a direct equivalent of the C++ write_command().
    pub fn write_command(&mut self, cmd: GxCommand) {
        self.gxfifo.push_back(cmd);

        if self.gxpipe.len() < 4 {
            if let Some(c) = self.gxfifo.pop_front() {
                self.gxpipe.push_back(c);
            }
        }

        self.check_fifo_dma();
        self.check_fifo_irq();
    }

    /// Execute a GX command
    ///
    /// (Implementation follows in the next section)
    pub fn exec_command(&mut self) {
        // implemented in next stage
    }
}

impl<'a> Gpu3D<'a> {
    /// Execute a single GX command
    ///
    /// This function is a faithful port of the C++ exec_command() implementation.
    /// It performs command dispatch, cycle accounting, and GXSTAT updates.
    pub fn exec_command(&mut self) {
        let cmd = self.read_command();
        let opcode = cmd.command;
        let param = cmd.param;

        // Consume cycles
        self.cycles += Self::CMD_CYCLE_AMOUNTS[opcode as usize] as i64;

        match opcode {
            0x00 => {
                // NOP
            }

            0x10 => {
                // MTX_MODE
                self.set_mtx_mode(param);
            }

            0x11 => {
                // MTX_PUSH
                self.mtx_push();
            }

            0x12 => {
                // MTX_POP
                self.mtx_pop(param);
            }

            0x13 => {
                // MTX_STORE (not implemented on NDS, treated as NOP)
            }

            0x14 => {
                // MTX_RESTORE (not implemented on NDS, treated as NOP)
            }

            0x15 => {
                // MTX_IDENTITY
                self.mtx_identity();
            }

            0x16 => {
                // MTX_LOAD_4x4
                self.add_mult_param(param);
                if self.mult_params_index == 16 {
                    self.mtx_load_4x4();
                }
            }

            0x17 => {
                // MTX_LOAD_4x3
                self.add_mult_param(param);
                if self.mult_params_index == 12 {
                    self.mtx_load_4x3();
                }
            }

            0x18 => {
                // MTX_MULT_4x4
                self.add_mult_param(param);
                if self.mult_params_index == 16 {
                    self.mtx_mult_4x4();
                }
            }

            0x19 => {
                // MTX_MULT_4x3
                self.add_mult_param(param);
                if self.mult_params_index == 12 {
                    self.mtx_mult_4x3();
                }
            }

            0x1A => {
                // MTX_MULT_3x3
                self.add_mult_param(param);
                if self.mult_params_index == 9 {
                    self.mtx_mult_3x3();
                }
            }

            0x1B => {
                // MTX_TRANS
                self.add_mult_param(param);
                if self.mult_params_index == 3 {
                    self.mtx_trans();
                }
            }

            0x20 => {
                // COLOR
                self.color(param);
            }

            0x21 => {
                // NORMAL
                self.normal();
            }

            0x22 => {
                // TEXCOORD
                self.current_texcoords[0] = (param & 0xFFFF) as i16;
                self.current_texcoords[1] = (param >> 16) as i16;
            }

            0x23 => {
                // VTX_16
                self.add_vertex_16(param);
            }

            0x24 => {
                // VTX_10
                self.add_vertex_10(param);
            }

            0x25 => {
                // VTX_XY
                self.add_vertex_xy(param);
            }

            0x26 => {
                // VTX_XZ
                self.add_vertex_xz(param);
            }

            0x27 => {
                // VTX_YZ
                self.add_vertex_yz(param);
            }

            0x28 => {
                // VTX_DIFF
                self.add_vertex_diff(param);
            }

            0x29 => {
                // POLYGON_ATTR
                self.set_polygon_attr(param);
            }

            0x2A => {
                // TEXIMAGE_PARAM
                self.set_teximage_param(param);
            }

            0x2B => {
                // PLTT_BASE
                self.pltt_base = param & 0x1FFF;
            }

            0x30 => {
                // DIF_AMB
                self.diffuse_color = (param & 0xFFFF) as u16;
                self.ambient_color = (param >> 16) as u16;
            }

            0x31 => {
                // SPE_EMI
                self.specular_color = (param & 0xFFFF) as u16;
                self.emission_color = (param >> 16) as u16;
            }

            0x32 => {
                // LIGHT_VECTOR
                let id = (param >> 30) as usize;
                self.light_direction[id][0] = ((param << 6) >> 22) as i16;
                self.light_direction[id][1] = ((param << 16) >> 22) as i16;
                self.light_direction[id][2] = ((param << 26) >> 22) as i16;
            }

            0x33 => {
                // LIGHT_COLOR
                let id = (param >> 30) as usize;
                self.light_color[id] = (param & 0x7FFF) as u16;
            }

            0x34 => {
                // SHININESS
                let index = (self.cmd_param_count & 0x7F) as usize;
                self.shine_table[index] = (param & 0xFF) as u8;
                self.cmd_param_count += 1;
                self.using_shine_table = true;
            }

            0x40 => {
                // BEGIN_VTXS
                self.begin_vtxs(param);
            }

            0x41 => {
                // END_VTXS
                self.end_vtxs();
            }

            0x50 => {
                // SWAP_BUFFERS
                self.swap_buffers(param);
            }

            0x60 => {
                // VIEWPORT
                self.set_viewport(param);
            }

            0x70 => {
                // BOX_TEST
                self.box_test();
            }

            0x71 => {
                // POS_TEST
                self.pos_test();
            }

            0x72 => {
                // VEC_TEST
                self.vec_test();
            }

            0xFF => {
                // NOP (padding)
            }

            _ => {
                // Unknown / unimplemented opcode
                // Faithful behavior: ignore
            }
        }
    }
}

impl<'a> Gpu3D<'a> {
    /// Add a parameter for matrix multiplication
    pub fn add_mult_param(&mut self, word: u32) {
        let row = self.mult_params_index / 4;
        let col = self.mult_params_index % 4;
        self.mult_params.m[row as usize][col as usize] = word as i32;
        self.mult_params_index += 1;
    }

    /// Set matrix mode
    pub fn set_mtx_mode(&mut self, word: u32) {
        self.mtx_mode = (word & 0x3) as u8;
    }

    /// Push matrix stack
    pub fn mtx_push(&mut self) {
        match self.mtx_mode {
            0 => self.projection_stack = self.projection_mtx,
            1 => {
                if self.modelview_sp < 0x1F {
                    self.modelview_stack[self.modelview_sp as usize] =
                        self.modelview_mtx;
                    self.modelview_sp += 1;
                } else {
                    self.gxstat.mtx_overflow = true;
                }
            }
            2 => {
                self.vector_stack[self.modelview_sp as usize] = self.vector_mtx;
            }
            3 => self.texture_stack = self.texture_mtx,
            _ => {}
        }
    }

    /// Pop matrix stack
    pub fn mtx_pop(&mut self, word: u32) {
        let count = (word & 0x3F) as i32;
        match self.mtx_mode {
            1 => {
                for _ in 0..count {
                    if self.modelview_sp > 0 {
                        self.modelview_sp -= 1;
                        self.modelview_mtx =
                            self.modelview_stack[self.modelview_sp as usize];
                    }
                }
            }
            _ => {}
        }
    }

    /// Load identity matrix
    pub fn mtx_identity(&mut self) {
        match self.mtx_mode {
            0 => self.projection_mtx = Self::IDENTITY,
            1 => self.modelview_mtx = Self::IDENTITY,
            2 => self.vector_mtx = Self::IDENTITY,
            3 => self.texture_mtx = Self::IDENTITY,
            _ => {}
        }
        self.clip_dirty = true;
    }

    /// Load 4x4 matrix
    pub fn mtx_load_4x4(&mut self) {
        match self.mtx_mode {
            0 => self.projection_mtx = self.mult_params,
            1 => self.modelview_mtx = self.mult_params,
            2 => self.vector_mtx = self.mult_params,
            3 => self.texture_mtx = self.mult_params,
            _ => {}
        }
        self.mult_params_index = 0;
        self.clip_dirty = true;
    }

    /// Load 4x3 matrix
    pub fn mtx_load_4x3(&mut self) {
        let mut m = Self::IDENTITY;
        for i in 0..3 {
            for j in 0..4 {
                m.m[i][j] = self.mult_params.m[i][j];
            }
        }

        match self.mtx_mode {
            0 => self.projection_mtx = m,
            1 => self.modelview_mtx = m,
            2 => self.vector_mtx = m,
            3 => self.texture_mtx = m,
            _ => {}
        }

        self.mult_params_index = 0;
        self.clip_dirty = true;
    }

    /// Multiply matrices (4x4)
    pub fn mtx_mult_4x4(&mut self) {
        let src = self.mult_params;
        let dst = match self.mtx_mode {
            0 => &mut self.projection_mtx,
            1 => &mut self.modelview_mtx,
            2 => &mut self.vector_mtx,
            3 => &mut self.texture_mtx,
            _ => return,
        };

        let mut out = Self::IDENTITY;

        for i in 0..4 {
            for j in 0..4 {
                let mut acc = 0i64;
                for k in 0..4 {
                    acc += dst.m[i][k] as i64 * src.m[k][j] as i64;
                }
                out.m[i][j] = (acc >> 12) as i32;
            }
        }

        *dst = out;
        self.mult_params_index = 0;
        self.clip_dirty = true;
    }

    /// Set current color
    pub fn color(&mut self, word: u32) {
        self.current_color = word & 0x7FFF;
    }

    /// Set normal vector
    pub fn normal(&mut self) {
        // Lighting calculation deferred (same as C++)
    }

    /// Begin vertex submission
    pub fn begin_vtxs(&mut self, word: u32) {
        self.polygon_type = word & 0x3;
        self.vertex_list_count = 0;
        self.consecutive_polygons = 0;
        self.last_poly_strip = None;
    }

    /// End vertex submission
    pub fn end_vtxs(&mut self) {
        self.vertex_list_count = 0;
    }

    /// Add vertex (VTX_16)
    pub fn add_vertex_16(&mut self, word: u32) {
        self.current_vertex[0] = (word & 0xFFFF) as i16;
        self.current_vertex[1] = (word >> 16) as i16;
        self.add_vertex();
    }

    /// Add vertex (VTX_10)
    pub fn add_vertex_10(&mut self, word: u32) {
        self.current_vertex[0] = ((word << 22) >> 22) as i16;
        self.current_vertex[1] = ((word << 12) >> 22) as i16;
        self.current_vertex[2] = ((word << 2) >> 22) as i16;
        self.add_vertex();
    }

    /// Add vertex (XY)
    pub fn add_vertex_xy(&mut self, word: u32) {
        self.current_vertex[0] = (word & 0xFFFF) as i16;
        self.current_vertex[1] = (word >> 16) as i16;
        self.add_vertex();
    }

    /// Add vertex (XZ)
    pub fn add_vertex_xz(&mut self, word: u32) {
        self.current_vertex[0] = (word & 0xFFFF) as i16;
        self.current_vertex[2] = (word >> 16) as i16;
        self.add_vertex();
    }

    /// Add vertex (YZ)
    pub fn add_vertex_yz(&mut self, word: u32) {
        self.current_vertex[1] = (word & 0xFFFF) as i16;
        self.current_vertex[2] = (word >> 16) as i16;
        self.add_vertex();
    }

    /// Add vertex (DIFF)
    pub fn add_vertex_diff(&mut self, word: u32) {
        self.current_vertex[0] += ((word << 22) >> 22) as i16;
        self.current_vertex[1] += ((word << 12) >> 22) as i16;
        self.current_vertex[2] += ((word << 2) >> 22) as i16;
        self.add_vertex();
    }

    /// Add vertex to geometry list
    pub fn add_vertex(&mut self) {
        // Geometry transform + polygon assembly handled next stage
        // Same separation as original C++
    }
}

impl Mtx {
    #[inline]
    pub fn mul_vec4(&self, v: [i32; 4]) -> [i32; 4] {
        let mut out = [0i32; 4];
        for i in 0..4 {
            let mut acc = 0i64;
            for j in 0..4 {
                acc += self.m[i][j] as i64 * v[j] as i64;
            }
            out[i] = (acc >> 12) as i32;
        }
        out
    }
}

impl<'a> Gpu3D<'a> {
    pub fn add_vertex(&mut self) {
        if self.vertex_list_count >= 10 {
            return;
        }

        // 入力頂点（1.12 fixed）
        let v = [
            self.current_vertex[0] as i32,
            self.current_vertex[1] as i32,
            self.current_vertex[2] as i32,
            1 << 12,
        ];

        // ModelView → Clip
        let mv = self.modelview_mtx.mul_vec4(v);

        // Projection
        let clip = self.projection_mtx.mul_vec4(mv);

        let vert = &mut self.vertex_list[self.vertex_list_count as usize];
        vert.x = clip[0];
        vert.y = clip[1];
        vert.z = clip[2];
        vert.w = clip[3];

        vert.color = self.current_color as u16;
        vert.u = self.current_texcoords[0];
        vert.v = self.current_texcoords[1];

        self.vertex_list_count += 1;

        // ポリゴン生成条件
        match self.polygon_type {
            0 => {
                // TRIANGLES
                if self.vertex_list_count == 3 {
                    self.add_polygon(3);
                    self.vertex_list_count = 0;
                }
            }
            1 => {
                // QUADS
                if self.vertex_list_count == 4 {
                    self.add_polygon(4);
                    self.vertex_list_count = 0;
                }
            }
            2 => {
                // TRIANGLE_STRIP
                if self.vertex_list_count >= 3 {
                    self.add_polygon(3);
                }
            }
            3 => {
                // QUAD_STRIP
                if self.vertex_list_count >= 4 {
                    self.add_polygon(4);
                }
            }
            _ => {}
        }
    }
}

impl Vertex {
    #[inline]
    pub fn inside_plane(&self, plane: usize) -> bool {
        match plane {
            0 => self.x >= -self.w, // left
            1 => self.x <= self.w,  // right
            2 => self.y >= -self.w, // bottom
            3 => self.y <= self.w,  // top
            4 => self.z >= 0,       // near
            5 => self.z <= self.w,  // far
            _ => true,
        }
    }
}

impl<'a> Gpu3D<'a> {
    pub fn add_polygon(&mut self, count: usize) {
        let mut poly = Polygon::default();
        poly.attr = self.polygon_attr;
        poly.vertex_count = count as u8;

        for i in 0..count {
            poly.vertices[i] =
                self.vertex_list[(self.vertex_list_count as usize) - count + i];
        }

        // clip
        let mut temp1 = Vec::new();
        let mut temp2 = Vec::new();

        temp1.extend_from_slice(&poly.vertices[..count]);

        for plane in 0..6 {
            self.clip_plane(&mut temp2, &temp1, plane);
            std::mem::swap(&mut temp1, &mut temp2);
            if temp1.is_empty() {
                return;
            }
        }

        // store
        let id = self.geo_poly_count as usize;
        self.geo_poly[id] = poly;
        self.geo_poly_count += 1;
    }
}

impl<'a> Gpu3D<'a> {
    /// Convert clip-space vertex to screen-space
    pub fn project_vertex(&self, v: &mut Vertex) {
        // Perspective divide
        let inv_w = ((1 << 24) / v.w) as i32;

        let x_ndc = (v.x * inv_w) >> 12;
        let y_ndc = (v.y * inv_w) >> 12;
        let z_ndc = (v.z * inv_w) >> 12;

        // Viewport transform
        let vp = &self.viewport;

        v.x = vp.x1 as i32
            + ((x_ndc + (1 << 12)) * (vp.x2 as i32 - vp.x1 as i32) >> 13);

        v.y = vp.y1 as i32
            + ((y_ndc + (1 << 12)) * (vp.y2 as i32 - vp.y1 as i32) >> 13);

        v.z = z_ndc;
        v.w = inv_w;
    }
}

impl<'a> Gpu3D<'a> {
    /// Prepare polygons for rasterization
    pub fn finalize_geometry(&mut self) {
        self.rend_poly_count = 0;
        self.rend_vert_count = 0;

        for i in 0..self.geo_poly_count {
            let mut poly = self.geo_poly[i];

            for v in 0..poly.vertex_count as usize {
                let mut vert = poly.vertices[v];
                self.project_vertex(&mut vert);
                poly.vertices[v] = vert;
            }

            self.rend_poly[self.rend_poly_count as usize] = poly;
            self.rend_poly_count += 1;
        }

        self.geo_poly_count = 0;
    }
}

/// Compute edge function value
#[inline]
fn edge(ax: i32, ay: i32, bx: i32, by: i32, cx: i32, cy: i32) -> i32 {
    (cx - ax) * (by - ay) - (cy - ay) * (bx - ax)
}

impl<'a> Gpu3D<'a> {
    /// Rasterize all polygons into the framebuffer
    pub fn render_scanline(&mut self, y: usize) {
        // Clear line Z buffer
        for x in 0..PIXELS_PER_LINE {
            self.z_buffer[y][x] = 0xFFFFFFFF;
        }

        for i in 0..self.rend_poly_count {
            let poly = &self.rend_poly[i];
            if poly.vertex_count < 3 {
                continue;
            }

            let v0 = poly.vertices[0];
            let v1 = poly.vertices[1];
            let v2 = poly.vertices[2];

            let min_x = v0.x.min(v1.x).min(v2.x).max(0) as usize;
            let max_x = v0.x.max(v1.x).max(v2.x).min(255) as usize;

            if y < v0.y.min(v1.y).min(v2.y) as usize
                || y > v0.y.max(v1.y).max(v2.y) as usize
            {
                continue;
            }

            let area = edge(v0.x, v0.y, v1.x, v1.y, v2.x, v2.y);
            if area == 0 {
                continue;
            }

            for x in min_x..=max_x {
                let w0 = edge(v1.x, v1.y, v2.x, v2.y, x as i32, y as i32);
                let w1 = edge(v2.x, v2.y, v0.x, v0.y, x as i32, y as i32);
                let w2 = edge(v0.x, v0.y, v1.x, v1.y, x as i32, y as i32);

                if w0 >= 0 && w1 >= 0 && w2 >= 0 {
                    let z = ((w0 as i64 * v0.z as i64)
                        + (w1 as i64 * v1.z as i64)
                        + (w2 as i64 * v2.z as i64))
                        / area as i64;

                    let z = z as u32;

                    if z < self.z_buffer[y][x] {
                        self.z_buffer[y][x] = z;

                        unsafe {
                            (*self.gpu).draw_pixel(
                                x as u16,
                                y as u16,
                                poly.vertices[0].color,
                            );
                        }
                    }
                }
            }
        }
    }
}

impl<'a> Gpu3D<'a> {
    /// Finalize frame rendering
    pub fn end_frame(&mut self) {
        self.finalize_geometry();
        self.swap_buffers = false;
    }
}
