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
use rendering::{FrameBufferAttr, FrameBufferPixel};

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
    /// EDGE_COLOR (4000330h-33Fh): 8 BGR555 colors, one per group of 8
    /// polygon IDs, used by edge marking (DISP3DCNT bit 5).
    ///
    /// Skipped by the savestate; games reprogram it as part of their
    /// per-frame 3D setup, matching `alpha_test_ref`.
    #[savestate(skip)]
    edge_color: [Color; 8],
    /// CLRIMAGE_OFFSET (4000356h): (x, y) scroll offset into the rear-plane
    /// bitmap when DISP3DCNT bit 14 (rear-plane bitmap mode) is set.
    #[savestate(skip)]
    clrimage_offset: (u8, u8),
    /// FOG_COLOR (4000358h): fog RGBA, used by the fog post-pass.
    #[savestate(skip)]
    fog_color: FogColor,
    /// FOG_OFFSET (400035Ch): 15-bit depth at which fog density starts
    /// ramping up from FOG_TABLE[0].
    #[savestate(skip)]
    fog_offset: u16,
    /// FOG_TABLE (4000360h-37Fh): 32 7-bit density entries.
    #[savestate(skip)]
    fog_table: [u8; 32],
    /// ALPHA_TEST_REF (4000340h): fragments whose alpha is less than or equal
    /// to this 5-bit reference are discarded while DISP3DCNT bit 2 is set.
    ///
    /// GBATEK "DS 3D Display Control": <https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol>
    ///
    /// Skipped by the savestate so that states written before the alpha test
    /// existed still load; games reprogram it as part of their per-frame 3D
    /// setup.
    #[savestate(skip)]
    alpha_test_ref: u8,
    #[load(with = "save.load()?", with_in_place = "*frame_buffer = save.load()?")]
    frame_buffer: Vec<FrameBufferPixel>,
    /// Per-pixel render attributes (fog flag, opaque/translucent polygon IDs),
    /// rebuilt from scratch by `render()` every frame.
    ///
    /// Skipped by the savestate for the same reason as `alpha_test_ref`: this
    /// is pure intra-frame state that never needs to survive a state load. See
    /// `docs/design/3d-fog-and-rendering-fixes-design.md` §2.
    #[savestate(skip)]
    attr_buffer: Vec<FrameBufferAttr>,
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
            edge_color: [Color::new5(0, 0, 0); 8],
            clrimage_offset: (0, 0),
            fog_color: FogColor::new(),
            fog_offset: 0,
            fog_table: [0; 32],
            alpha_test_ref: 0,
            frame_buffer: vec![FrameBufferPixel::new(); GPU::WIDTH * GPU::HEIGHT],
            attr_buffer: vec![FrameBufferAttr::default(); GPU::WIDTH * GPU::HEIGHT],
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
            0x4A4..=0x4A7 => {
                // POLYGON_ATTR (Cmd 29h) is a write-only Geometry Engine command.
                // GBATEK marks this register as (W) only, and melonDS only implements
                // the write path (GPU3D.cpp::Write8/16/32). Reads are not expected.
                //
                // References:
                // - GBATEK: 040004A4h Cmd 29h POLYGON_ATTR (W)
                // https://problemkaputt.de/gbatek.htm#ds3diomap:~:text=40004A4h%2029h%201%20%201%20%20%20POLYGON_ATTR%20-%20Set%20Polygon%20Attributes%20(W)
                // - melonDS: src/GPU3D.cpp (command dispatcher)
                // https://github.com/Thoughts-with-friends/folk-melonDS/blob/master/src/GPU3D.cpp#L2550
                warn!("Read from write-only POLYGON_ATTR register");
                0 // Avoid Pokemon Black2 start crash
            }
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
            // EDGE_COLOR: 8 BGR555 colors, 2 bytes each.
            //
            // GBATEK "DS 3D Toon, Edge, Fog, Alpha Blending, Anti-aliasing":
            // <https://problemkaputt.de/gbatek-ds-3d-toon-edge-fog-alpha-blending-anti-aliasing.htm>
            0x330..=0x33F => {
                let offset = (addr & 0xFFF) as usize - 0x330;
                let index = offset / 2;
                let old_value = self.edge_color[index].as_u16();
                self.edge_color[index] = Color::from(if offset & 0x1 == 0 {
                    old_value & !0x00FF | (value as u16)
                } else {
                    old_value & !0xFF00 | (value as u16) << 8
                });
            }
            // ALPHA_TEST_REF: only the low 5 bits are meaningful; the register
            // is 32 bit wide but bits 5-31 are unused.
            0x340 => self.alpha_test_ref = value & 0x1F,
            0x341..=0x343 => (),
            0x350..=0x353 => self.clear_color.write(scheduler, addr as usize & 0x3, value),
            0x354..=0x355 => self.clear_depth.write(scheduler, addr as usize & 0x1, value),
            // CLRIMAGE_OFFSET: (X, Y) scroll offset into the rear-plane bitmap
            // (DISP3DCNT bit 14). Only meaningful in rear-plane bitmap mode.
            0x356 => self.clrimage_offset.0 = value,
            0x357 => self.clrimage_offset.1 = value,
            0x358..=0x35B => self.fog_color.write(addr as usize & 0x3, value),
            // FOG_OFFSET: 15-bit unsigned depth at which fog starts ramping
            // from FOG_TABLE[0].
            0x35C => self.fog_offset = self.fog_offset & !0xFF | value as u16,
            0x35D => self.fog_offset = self.fog_offset & !0x7F00 | (value as u16) << 8 & 0x7F00,
            // FOG_TABLE: 32 density entries, one byte each; only the low 7
            // bits are meaningful.
            0x360..=0x37F => self.fog_table[(addr & 0x1F) as usize] = value & 0x7F,
            0x380..=0x3BF => {
                self.write_toon_table(addr as usize & (2 * self.toon_table.len() - 1), value)
            }
            0x600..=0x603 => self.write_gxstat(interrupts, (addr as usize) & 0x3, value),
            _ => warn!("Ignoring Engine3D Write 0x{:08X} = {:02X}", addr, value),
        }
    }
}

#[cfg(test)]
mod fog_integration_tests {
    use super::*;
    use crate::hw::gpu::VRAM;

    /// Submits a single full-screen, single-color quad through the real
    /// GXFIFO command path (VIEWPORT, COLOR, POLYGON_ATTR, BEGIN/END_VTXS,
    /// four VTX_XY, SWAP_BUFFERS), then calls the real `render()` used by
    /// the emulator's frame loop. Exercises the exact same code paths a
    /// game's GX command stream would (register decoding, the per-pixel
    /// `attr_buffer` wiring in `render_polygon`, and the `apply_fog`
    /// post-pass), rather than calling `apply_fog` directly.
    fn render_full_screen_green_quad(engine: &mut Engine3D, fog_enable: bool) -> u16 {
        let mut interrupts = InterruptRequest::empty();
        let mut scheduler = Scheduler::new();
        let vram = VRAM::new();

        // CLEAR_DEPTH defaults to 0, which encodes to a *near*-plane depth
        // (`ClearDepth::depth()` = 0x1FF); a quad sitting at clip Z=0 (the
        // middle of the view volume) is farther than that and would fail
        // the depth test against the background. Set it to the far plane
        // (max 15-bit value) so the quad draws, matching how a real game
        // programs it before submitting opaque geometry.
        engine.write_register(&mut interrupts, &mut scheduler, 0x4000354, 0xFF);
        engine.write_register(&mut interrupts, &mut scheduler, 0x4000355, 0x7F);

        // Full-screen viewport (X1=0,Y1=0,X2=255,Y2=191), same encoding
        // exercised by `registers::tests::full_screen`.
        engine.write_geometry_command(&mut interrupts, 0x4000580, 0xBF_FF_00_00);

        // COLOR: RGB555 pure green (r=0,g=31,b=0).
        engine.write_geometry_command(&mut interrupts, 0x4000480, 0x03E0);

        // POLYGON_ATTR: render both front and back (winding is not
        // meaningful for this synthetic full-screen quad), alpha=0x1F,
        // polygon_id=1, fog_enable per the `fog_enable` argument.
        let polygon_attr = (0x3 << 6) | ((fog_enable as u32) << 15) | (0x1F << 16) | (1 << 24);
        engine.write_geometry_command(&mut interrupts, 0x40004A4, polygon_attr);

        // BEGIN_VTXS(Quad=1), four corners of the view volume (Vtx_XY at
        // clip Z/W = 0/1, matching `registers::tests`'s "clip_mat is
        // identity" setup), END_VTXS, SWAP_BUFFERS(no manual sort, no
        // W-buffer).
        engine.write_geometry_command(&mut interrupts, 0x4000500, 1);
        let vtx_xy = |x: i16, y: i16| ((y as u16 as u32) << 16) | (x as u16 as u32);
        engine.write_geometry_command(&mut interrupts, 0x4000494, vtx_xy(-4096, 4096));
        engine.write_geometry_command(&mut interrupts, 0x4000494, vtx_xy(4096, 4096));
        engine.write_geometry_command(&mut interrupts, 0x4000494, vtx_xy(4096, -4096));
        engine.write_geometry_command(&mut interrupts, 0x4000494, vtx_xy(-4096, -4096));
        engine.write_geometry_command(&mut interrupts, 0x4000504, 0);
        engine.write_geometry_command(&mut interrupts, 0x4000540, 0);

        engine.render(&vram, true);
        engine.pixel_color(GPU::WIDTH * (GPU::HEIGHT / 2) + GPU::WIDTH / 2)
    }

    /// With fog disabled, the quad renders as the vertex color exactly
    /// (Modulation mode with no texture passes the vertex color through
    /// unchanged) - this is the baseline the fog test below is compared
    /// against.
    #[test]
    fn quad_without_fog_renders_pure_vertex_color() {
        let mut engine = Engine3D::new();
        let pixel = render_full_screen_green_quad(&mut engine, true);
        assert_eq!(pixel, Color::new6(0, 63, 0).as_u16() | 0x8000, "expected opaque pure green");
    }

    /// End-to-end fog test through the real register-write and render path.
    ///
    /// FOG_TABLE is filled with a uniform density (0x40 = 64/128), which
    /// makes the interpolated density exactly 64 for every depth
    /// (interpolating between two equal table entries can't produce
    /// anything else), independent of the quad's actual Z. That turns the
    /// blend formula `(new*density + old*(128-density)) >> 7` into an exact
    /// per-channel average of the vertex color and FOG_COLOR, which can be
    /// checked precisely instead of approximately.
    ///
    /// See `docs/design/3d-fog-and-rendering-fixes-design.md` §4.
    #[test]
    fn quad_with_uniform_density_fog_blends_toward_fog_color() {
        let mut engine = Engine3D::new();
        let mut interrupts = InterruptRequest::empty();
        let mut scheduler = Scheduler::new();

        // DISP3DCNT: fog master enable (bit 7), color+alpha mode (bit 6 = 0).
        engine.disp3dcnt.fog_master_enable = true;
        engine.disp3dcnt.fog_alpha_only = false;

        // FOG_COLOR: RGB555 pure blue, full alpha (r=0,g=0,b=31,a=31).
        // Byte layout per `FogColor::write`: byte0 = r + g_lo, byte1 =
        // g_hi + b, byte2 = a, byte3 = unused.
        engine.write_register(&mut interrupts, &mut scheduler, 0x4000358, 0x00);
        engine.write_register(&mut interrupts, &mut scheduler, 0x4000359, 0x7C); // b = 0x1F
        engine.write_register(&mut interrupts, &mut scheduler, 0x400035A, 0x1F); // a = 0x1F
        engine.write_register(&mut interrupts, &mut scheduler, 0x400035B, 0x00);

        // FOG_OFFSET = 0: fog applies starting at the nearest depth.
        engine.write_register(&mut interrupts, &mut scheduler, 0x400035C, 0);
        engine.write_register(&mut interrupts, &mut scheduler, 0x400035D, 0);

        // FOG_TABLE[0..32] = 0x40 uniformly.
        for addr in 0x4000360..=0x400037F {
            engine.write_register(&mut interrupts, &mut scheduler, addr, 0x40);
        }

        let pixel = render_full_screen_green_quad(&mut engine, true);

        // Per-channel average of green (0,63,0) and blue (0,0,63) in 6-bit
        // space is (0,31,31); alpha averages to 63 (opaque).
        assert_eq!(
            pixel,
            Color::new6(0, 31, 31).as_u16() | 0x8000,
            "fog should blend the quad exactly halfway toward FOG_COLOR"
        );
    }

    /// A polygon whose POLYGON_ATTR fog-enable bit is clear must not be
    /// fogged even when DISP3DCNT's fog master enable is set - the fog flag
    /// is per polygon, not global.
    #[test]
    fn polygon_without_fog_enable_bit_is_not_fogged() {
        let mut engine = Engine3D::new();
        let mut interrupts = InterruptRequest::empty();
        let mut scheduler = Scheduler::new();

        engine.disp3dcnt.fog_master_enable = true;
        engine.write_register(&mut interrupts, &mut scheduler, 0x4000359, 0x7C); // FOG_COLOR b = 0x1F
        engine.write_register(&mut interrupts, &mut scheduler, 0x400035A, 0x1F); // FOG_COLOR a = 0x1F
        for addr in 0x4000360..=0x400037F {
            engine.write_register(&mut interrupts, &mut scheduler, addr, 0x40);
        }

        let pixel = render_full_screen_green_quad(&mut engine, false);
        assert_eq!(pixel, Color::new6(0, 63, 0).as_u16() | 0x8000, "unfogged polygon stays green");
    }
}
