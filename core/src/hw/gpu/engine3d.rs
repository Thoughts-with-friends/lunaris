//! 3D engine: geometry engine (command FIFO, matrices, lighting, textures)
//! plus a software rendering engine producing a 256×192 frame buffer.
//!
//! GBATEK references:
//! - 3D overview (geometry + rendering engine split):
//!   <https://problemkaputt.de/gbatek.htm#ds3doverview>
//! - 3D I/O map: <https://problemkaputt.de/gbatek.htm#ds3diomap>
//! - Geometry commands / GXFIFO: <https://problemkaputt.de/gbatek.htm#ds3dgeometrycommands>
//! - GXSTAT status register: <https://problemkaputt.de/gbatek.htm#ds3dstatus>
//! - Matrix stacks: <https://problemkaputt.de/gbatek.htm#ds3dmatrixstack>

use std::collections::VecDeque;

use super::{GPU, InterruptRequest, Scheduler};
use crate::hw::mem::IORegister;

mod geometry;
mod math;
mod registers;
mod rendering;

use geometry::*;
use math::{FixedPoint, Matrix};
use registers::*;
use rendering::FrameBufferPixel;

#[derive(emu_utils::Savestate)]
pub struct Engine3D {
    pub bus_stalled: bool,
    // Registers
    pub disp3dcnt: DISP3DCNT,
    gxstat: GXSTAT,
    // Geometry Engine
    prev_command: GeometryCommand, // Verification for Geometry Commands
    packed_commands: u32,
    cur_command: GeometryCommand, // Current Packed Command processing
    num_params: usize,
    params_processed: usize,
    /// `Vec`/`VecDeque::load_in_place` does not consume the stored length
    /// prefix, so route through `Loadable` instead.
    /// See `docs/design/savestate-and-video-design.md`.
    #[load(with = "save.load()?", with_in_place = "*params = save.load()?")]
    params: Vec<u32>,
    #[load(with = "save.load()?", with_in_place = "*gxfifo = save.load()?")]
    gxfifo: VecDeque<GeometryCommandEntry>,
    // Matrices
    mtx_mode: MatrixMode,
    cur_proj: Matrix,
    cur_pos: Matrix,
    cur_vec: Matrix,
    cur_tex: Matrix,
    proj_stack_sp: u8,
    pos_vec_stack_sp: u8,
    tex_stack_sp: u8,
    proj_stack: [Matrix; 1], // Projection Stack
    pos_stack: [Matrix; 31], // Coordinate Stack
    vec_stack: [Matrix; 31], // Directional Stack
    tex_stack: [Matrix; 1],  // Texture Stack
    // Rendering Engine
    frame_params: FrameParams,
    next_frame_params: FrameParams,
    viewport: Viewport,
    clear_color: ClearColor,
    clear_depth: ClearDepth,
    #[load(with = "save.load()?", with_in_place = "*frame_buffer = save.load()?")]
    frame_buffer: Vec<FrameBufferPixel>,
    polygons_submitted: bool,
    // Polygons
    polygon_attrs: PolygonAttributes,
    polygon_attrs_latch: PolygonAttributes,
    vertex_primitive: VertexPrimitive,
    prev_pos: [FixedPoint; 3],
    swap_verts: bool,
    clip_mat: Matrix,
    #[load(with = "save.load()?", with_in_place = "*cur_poly_verts = save.load()?")]
    cur_poly_verts: Vec<Vertex>,
    #[load(with = "save.load()?", with_in_place = "*vertices = save.load()?")]
    vertices: Vec<Vertex>,
    #[load(with = "save.load()?", with_in_place = "*polygons = save.load()?")]
    polygons: Vec<Polygon>,
    #[load(with = "save.load()?", with_in_place = "*original_verts = save.load()?")]
    original_verts: Vec<(Matrix, [FixedPoint; 3])>,
    // Lighting
    lights: [Light; 4],
    material: Material,
    color: Color,
    // Textures
    tex_params: TextureParams,
    palette_base: usize,
    raw_tex_coord: [i16; 2], // 1 + 11 + 4 fixed point
    tex_coord: [i16; 2],     // 1 + 11 + 4 fixed point
    // Toon
    toon_table: [Color; 0x20],
}

impl Engine3D {
    const FIFO_LEN: usize = 256;

    pub fn new() -> Self {
        Engine3D {
            bus_stalled: false,
            // Registers
            disp3dcnt: DISP3DCNT::new(),
            gxstat: GXSTAT::new(),
            // Geometry Engine
            prev_command: GeometryCommand::Unimplemented,
            packed_commands: 0,
            cur_command: GeometryCommand::Unimplemented,
            num_params: 0,
            params_processed: 0,
            params: Vec::new(),
            gxfifo: VecDeque::with_capacity(256),
            // Matrices
            mtx_mode: MatrixMode::Proj,
            cur_proj: Matrix::identity(),
            cur_pos: Matrix::identity(),
            cur_vec: Matrix::identity(),
            cur_tex: Matrix::identity(),
            proj_stack_sp: 0,
            pos_vec_stack_sp: 0,
            tex_stack_sp: 0,
            proj_stack: [Matrix::identity(); 1], // Projection Stack
            pos_stack: [Matrix::identity(); 31], // Coordinate Stack
            vec_stack: [Matrix::identity(); 31], // Directional Stack
            tex_stack: [Matrix::identity(); 1],  // Texture Stack
            // Rendering Engine
            frame_params: FrameParams::new(),
            next_frame_params: FrameParams::new(),
            viewport: Viewport::new(),
            clear_color: ClearColor::new(),
            clear_depth: ClearDepth::new(),
            frame_buffer: vec![FrameBufferPixel::new(); GPU::WIDTH * GPU::HEIGHT],
            polygons_submitted: false,
            // Polygons
            polygon_attrs: PolygonAttributes::new(),
            polygon_attrs_latch: PolygonAttributes::new(),
            vertex_primitive: VertexPrimitive::Triangles,
            prev_pos: [FixedPoint::zero(); 3],
            swap_verts: false,
            clip_mat: Matrix::identity(),
            cur_poly_verts: Vec::with_capacity(10),
            vertices: Vec::new(),
            polygons: Vec::new(),
            original_verts: Vec::new(),
            // Lighting
            lights: [Light::new(); 4],
            material: Material::new(),
            color: Color::new5(0, 0, 0),
            // Textures
            tex_params: TextureParams::new(),
            palette_base: 0,
            raw_tex_coord: [0; 2], // 1 + 11 + 4 fixed point
            tex_coord: [0; 2],     // 1 + 11 + 4 fixed point
            // Toon
            toon_table: [Color::new5(0, 0, 0); 0x20],
        }
    }

    /// Raises the geometry-command-FIFO IRQ according to GXSTAT bits 30-31
    /// (never / less than half full / empty).
    ///
    /// GBATEK "GXSTAT Bit30-31 Command FIFO IRQ":
    /// <https://problemkaputt.de/gbatek.htm#ds3dstatus>
    pub fn check_interrupts(&self, interrupts: &mut InterruptRequest) {
        if match self.gxstat.command_fifo_irq {
            CommandFifoIRQ::Never => false,
            CommandFifoIRQ::LessHalf => self.gxfifo.len() < Engine3D::FIFO_LEN / 2,
            CommandFifoIRQ::Empty => self.gxfifo.is_empty(),
        } {
            *interrupts |= InterruptRequest::GEOMETRY_COMMAND_FIFO
        }
    }
}

impl Engine3D {
    /// Reads a byte from a 3D-engine I/O register (GXSTAT, RAM_COUNT,
    /// CLIPMTX_RESULT).
    ///
    /// GBATEK "DS 3D I/O Map": <https://problemkaputt.de/gbatek.htm#ds3diomap>
    /// and "DS 3D Status": <https://problemkaputt.de/gbatek.htm#ds3dstatus>
    pub fn read_register(&self, addr: u32) -> u8 {
        assert_eq!(addr >> 12, 0x04000);
        match addr & 0xFFF {
            0x4A4..=0x4A7 => 0, // TODO: Figure out what this should actually do
            0x600..=0x603 => self.read_gxstat((addr as usize) & 0x3),
            0x604..=0x607 => self.read_ram_count((addr as usize) & 0x3),
            0x640..=0x67F => self.read_clip_mat((addr as usize) & 0x3F),
            _ => {
                warn!("Ignoring Engine3D Read at 0x{:08X}", addr);
                0
            }
        }
    }

    /// Writes a byte to a 3D-engine I/O register (CLEAR_COLOR, CLEAR_DEPTH,
    /// TOON_TABLE, GXSTAT).
    ///
    /// GBATEK references:
    /// - Register map: <https://problemkaputt.de/gbatek.htm#ds3diomap>
    /// - CLEAR_COLOR / CLEAR_DEPTH / TOON_TABLE:
    ///   <https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing>
    pub fn write_register(
        &mut self,
        interrupts: &mut InterruptRequest,
        scheduler: &mut Scheduler,
        addr: u32,
        value: u8,
    ) {
        assert_eq!(addr >> 12, 0x04000);
        match addr & 0xFFF {
            0x350..=0x353 => self.clear_color.write(scheduler, addr as usize & 0x3, value),
            0x354..=0x355 => self.clear_depth.write(scheduler, addr as usize & 0x1, value),
            0x380..=0x3BF => {
                self.write_toon_table(addr as usize & (2 * self.toon_table.len() - 1), value)
            }
            0x600..=0x603 => self.write_gxstat(interrupts, (addr as usize) & 0x3, value),
            _ => warn!("Ignoring Engine3D Write 0x{:08X} = {:02X}", addr, value),
        }
    }
}
