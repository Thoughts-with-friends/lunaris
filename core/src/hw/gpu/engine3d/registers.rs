use super::{Color, Engine3D, GPU, IORegister, InterruptRequest, Scheduler, math::Vec4};

#[derive(emu_utils::Savestate)]
pub struct DISP3DCNT {
    pub texture_mapping: bool,
    pub highlight_shading: bool,
    pub alpha_test: bool,
    pub alpha_blending: bool,
    pub antia_aliasing: bool,
    pub edge_marking: bool,
    pub fog_alpha_only: bool,
    pub fog_master_enable: bool,
    pub fog_depth_shift: u8,
    pub color_buffer_underflow: bool,
    pub poly_vert_ram_overflow: bool,
    pub rear_plane_bitmap: bool,
}

impl DISP3DCNT {
    pub fn new() -> Self {
        DISP3DCNT {
            texture_mapping: false,
            highlight_shading: false,
            alpha_test: false,
            alpha_blending: false,
            antia_aliasing: false,
            edge_marking: false,
            fog_alpha_only: false,
            fog_master_enable: false,
            fog_depth_shift: 0,
            color_buffer_underflow: false,
            poly_vert_ram_overflow: false,
            rear_plane_bitmap: false,
        }
    }
}

impl IORegister for DISP3DCNT {
    fn read(&self, byte: usize) -> u8 {
        match byte {
            0 => {
                (self.fog_master_enable as u8) << 7
                    | (self.fog_alpha_only as u8) << 6
                    | (self.edge_marking as u8) << 5
                    | (self.antia_aliasing as u8) << 4
                    | (self.alpha_blending as u8) << 3
                    | (self.alpha_test as u8) << 2
                    | (self.highlight_shading as u8) << 1
                    | self.texture_mapping as u8
            }
            1 => {
                (self.rear_plane_bitmap as u8) << 6
                    | (self.poly_vert_ram_overflow as u8) << 5
                    | (self.color_buffer_underflow as u8) << 4
                    | self.fog_depth_shift
            }
            2 | 3 => 0,
            _ => unreachable!(),
        }
    }

    fn write(&mut self, _scheduler: &mut Scheduler, byte: usize, value: u8) {
        match byte {
            0 => {
                self.texture_mapping = value & 0x1 != 0;
                self.highlight_shading = value >> 1 & 0x1 != 0;
                self.alpha_test = value >> 2 & 0x1 != 0;
                self.alpha_blending = value >> 3 & 0x1 != 0;
                self.antia_aliasing = value >> 4 & 0x1 != 0;
                self.edge_marking = value >> 5 & 0x1 != 0;
                self.fog_alpha_only = value >> 6 & 0x1 != 0;
                self.fog_master_enable = value >> 7 & 0x1 != 0;
            }
            1 => {
                self.fog_depth_shift = value & 0xF;
                self.color_buffer_underflow = self.color_buffer_underflow && value >> 4 & 0x1 == 0;
                self.poly_vert_ram_overflow = self.poly_vert_ram_overflow && value >> 4 & 0x1 == 0;
                self.rear_plane_bitmap = value >> 6 & 0x1 != 0;
            }
            2 | 3 => (),
            _ => unreachable!(),
        }
    }
}

#[derive(emu_utils::Savestate)]
pub struct GXSTAT {
    pub test_busy: bool, // Box, Pos, Vector Test
    pub box_test_inside: bool,
    pub mat_stack_busy: bool,
    pub mat_stack_error: bool, // Overflow or Underflow
    pub geometry_engine_busy: bool,
    pub command_fifo_irq: CommandFifoIRQ,
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
pub enum CommandFifoIRQ {
    Never = 0,
    LessHalf = 1,
    Empty = 2,
}

impl From<u8> for CommandFifoIRQ {
    /// Value `3` is reserved per GBATEK "DS 3D Status" (`#ds3dstatus`).
    /// Hardware does not halt on reserved register values, so this is
    /// treated as `Never` rather than panicking.
    fn from(value: u8) -> Self {
        match value {
            0 => CommandFifoIRQ::Never,
            1 => CommandFifoIRQ::LessHalf,
            2 => CommandFifoIRQ::Empty,
            _ => CommandFifoIRQ::Never,
        }
    }
}

impl GXSTAT {
    pub fn new() -> Self {
        GXSTAT {
            test_busy: false, // Box, Pos, Vector Test
            box_test_inside: false,
            mat_stack_busy: false,
            mat_stack_error: false, // Overflow or Underflow
            geometry_engine_busy: false,
            command_fifo_irq: CommandFifoIRQ::from(0),
        }
    }
}

impl Engine3D {
    pub(super) fn read_gxstat(&self, byte: usize) -> u8 {
        match byte {
            0 => (self.gxstat.box_test_inside as u8) << 1 | (self.gxstat.test_busy as u8),
            1 => {
                (self.gxstat.mat_stack_error as u8) << 7
                    | (self.gxstat.mat_stack_busy as u8) << 6
                    | self.proj_stack_sp << 5
                    | self.pos_vec_stack_sp & 0x1F
            }
            2 => self.gxfifo.len() as u8,
            3 => {
                (self.gxstat.command_fifo_irq as u8) << 6
                    | (self.gxstat.geometry_engine_busy as u8) << 3
                    | ((self.gxfifo.is_empty()) as u8) << 2
                    | ((self.gxfifo.len() < Engine3D::FIFO_LEN / 2) as u8) << 1
                    | (self.gxfifo.len() >> 8) as u8
            }
            _ => unreachable!(),
        }
    }

    pub(super) fn write_gxstat(
        &mut self,
        interrupts: &mut InterruptRequest,
        byte: usize,
        value: u8,
    ) {
        match byte {
            0 | 2 => (), // Read Only
            1 => self.gxstat.mat_stack_error = self.gxstat.mat_stack_error && value & 0x80 == 0,
            3 => self.gxstat.command_fifo_irq = CommandFifoIRQ::from(value >> 6 & 0x3),
            _ => unreachable!(),
        }
        self.check_interrupts(interrupts);
    }

    pub(super) fn read_ram_count(&self, byte: usize) -> u8 {
        match byte {
            0 => self.polygons.len() as u8,
            1 => (self.polygons.len() >> 8) as u8,
            2 => self.vertices.len() as u8,
            3 => (self.vertices.len() >> 8) as u8,
            _ => unreachable!(),
        }
    }

    pub(super) fn read_clip_mat(&self, byte: usize) -> u8 {
        ((self.clip_mat[byte / 4].raw() as u32) >> (8 * (byte % 4))) as u8
    }

    pub(super) fn write_toon_table(&mut self, addr: usize, value: u8) {
        let index = addr / 2;
        let old_value = self.toon_table[index].as_u16();
        self.toon_table[index] = Color::from(if addr & 0x1 == 0 {
            old_value & !0x00FF | (value as u16)
        } else {
            old_value & !0xFF00 | (value as u16) << 8
        });
    }
}

#[derive(emu_utils::Savestate)]
pub struct ClearColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub fog: bool,
    pub a: u8,
    pub polygon_id: u8,
}

impl ClearColor {
    pub fn new() -> Self {
        ClearColor { r: 0, g: 0, b: 0, fog: false, a: 0, polygon_id: 0 }
    }
}

impl IORegister for ClearColor {
    fn read(&self, _byte: usize) -> u8 {
        0
    }

    fn write(&mut self, _scheduler: &mut Scheduler, byte: usize, value: u8) {
        match byte {
            0 => {
                self.r = value & 0x1F;
                self.g = self.g & !0x7 | (value >> 5) & 0x7;
            }
            1 => {
                self.g = self.g & !0x18 | (value << 3) & 0x18;
                self.b = value >> 2 & 0x1F;
                self.fog = value >> 7 & 0x1 != 0;
            }
            2 => self.a = value & 0x1F,
            3 => self.polygon_id = value & 0x3F,
            _ => unreachable!(),
        }
    }
}

/// FOG_COLOR (4000358h): fog RGBA used by the fog post-pass.
///
/// GBATEK "DS 3D Toon, Edge, Fog, Alpha Blending, Anti-aliasing":
/// <https://problemkaputt.de/gbatek-ds-3d-toon-edge-fog-alpha-blending-anti-aliasing.htm>
pub struct FogColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl FogColor {
    pub fn new() -> Self {
        FogColor { r: 0, g: 0, b: 0, a: 0 }
    }

    pub fn write(&mut self, byte: usize, value: u8) {
        match byte {
            0 => {
                self.r = value & 0x1F;
                self.g = self.g & !0x7 | (value >> 5) & 0x7;
            }
            1 => {
                self.g = self.g & !0x18 | (value << 3) & 0x18;
                self.b = value >> 2 & 0x1F;
            }
            2 => self.a = value & 0x1F,
            3 => (),
            _ => unreachable!(),
        }
    }
}

#[derive(emu_utils::Savestate)]
pub struct ClearDepth {
    depth: u16,
}

impl ClearDepth {
    pub fn new() -> Self {
        ClearDepth { depth: 0 }
    }

    pub fn depth(&self) -> u32 {
        (self.depth as u32) * 0x200 + 0x1FF
    }
}

impl IORegister for ClearDepth {
    fn read(&self, _byte: usize) -> u8 {
        0
    }

    fn write(&mut self, _scheduler: &mut Scheduler, byte: usize, value: u8) {
        match byte {
            0 => self.depth = self.depth & !0xFF | value as u16,
            1 => self.depth = self.depth & !0x7F00 | (value as u16) << 8 & 0x7F00,
            _ => unreachable!(),
        }
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
pub struct TextureParams {
    pub vram_offset: usize,
    pub repeat_s: bool,
    pub repeat_t: bool,
    pub size_s_shift: u32,
    pub size_t_shift: u32,
    pub flip_s: bool,
    pub flip_t: bool,
    pub size_s: usize,
    pub size_t: usize,
    pub format: TextureFormat,
    pub color0_transparent: bool,
    pub coord_transformation_mode: TexCoordTransformationMode,
}

impl TextureParams {
    pub fn new() -> Self {
        TextureParams {
            vram_offset: 0,
            repeat_s: false,
            repeat_t: false,
            flip_s: false,
            flip_t: false,
            size_s_shift: 0,
            size_t_shift: 0,
            size_s: 1,
            size_t: 1,
            format: TextureFormat::NoTexture,
            color0_transparent: false,
            coord_transformation_mode: TexCoordTransformationMode::None,
        }
    }

    pub fn write(&mut self, value: u32) {
        self.vram_offset = ((value as usize) & 0xFFFF) << 3;
        self.repeat_s = value >> 16 & 0x1 != 0;
        self.repeat_t = value >> 17 & 0x1 != 0;
        self.flip_s = value >> 18 & 0x1 != 0;
        self.flip_t = value >> 19 & 0x1 != 0;
        self.size_s_shift = 3 + (value >> 20 & 0x7);
        self.size_t_shift = 3 + (value >> 23 & 0x7);
        self.size_s = 1 << self.size_s_shift;
        self.size_t = 1 << self.size_t_shift;
        self.format = TextureFormat::from(value >> 26 & 0x7);
        self.color0_transparent = value >> 29 & 0x1 != 0;
        self.coord_transformation_mode = TexCoordTransformationMode::from(value >> 30 & 0x3);
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
pub enum TextureFormat {
    NoTexture = 0,
    A3I5 = 1,
    Palette4 = 2,
    Palette16 = 3,
    Palette256 = 4,
    Compressed = 5,
    A5I3 = 6,
    DirectColor = 7,
}

impl From<u32> for TextureFormat {
    fn from(value: u32) -> Self {
        match value {
            0 => TextureFormat::NoTexture,
            1 => TextureFormat::A3I5,
            2 => TextureFormat::Palette4,
            3 => TextureFormat::Palette16,
            4 => TextureFormat::Palette256,
            5 => TextureFormat::Compressed,
            6 => TextureFormat::A5I3,
            7 => TextureFormat::DirectColor,
            _ => unreachable!(),
        }
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, PartialEq)]
pub enum TexCoordTransformationMode {
    None = 0,
    TexCoord = 1,
    Normal = 2,
    Vertex = 3,
}

impl From<u32> for TexCoordTransformationMode {
    fn from(value: u32) -> Self {
        match value {
            0 => TexCoordTransformationMode::None,
            2 => TexCoordTransformationMode::Normal,
            1 => TexCoordTransformationMode::TexCoord,
            3 => TexCoordTransformationMode::Vertex,
            _ => todo!(),
        }
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
pub struct PolygonAttributes {
    pub lights_enabled: [bool; 4],
    pub mode: PolygonMode,
    pub render_back: bool,
    pub render_front: bool,
    pub set_depth_translucent: bool,
    pub render_far_plane_intersecting: bool,
    pub render_1dot_behind_depth: bool,
    pub depth_test_eq: bool,
    pub fog_enable: bool,
    pub alpha: u8,
    pub polygon_id: u8,
}

impl PolygonAttributes {
    pub fn new() -> Self {
        PolygonAttributes {
            lights_enabled: [false; 4],
            mode: PolygonMode::Modulation,
            render_back: false,
            render_front: false,
            set_depth_translucent: false,
            render_far_plane_intersecting: false,
            render_1dot_behind_depth: false,
            depth_test_eq: false,
            fog_enable: false,
            alpha: 0,
            polygon_id: 0,
        }
    }

    pub fn write(&mut self, value: u32) {
        self.lights_enabled[0] = value & 0x1 != 0;
        self.lights_enabled[1] = value >> 1 & 0x1 != 0;
        self.lights_enabled[2] = value >> 2 & 0x1 != 0;
        self.lights_enabled[3] = value >> 3 & 0x1 != 0;
        self.mode = PolygonMode::from(value >> 4 & 0x3);
        self.render_back = value >> 6 & 0x1 != 0;
        self.render_front = value >> 7 & 0x1 != 0;
        self.set_depth_translucent = value >> 11 & 0x1 != 0;
        self.render_far_plane_intersecting = value >> 12 & 0x1 != 0;
        self.render_1dot_behind_depth = value >> 13 & 0x1 != 0;
        self.depth_test_eq = value >> 14 & 0x1 != 0;
        self.fog_enable = value >> 15 & 0x1 != 0;
        self.alpha = (value >> 16 & 0x1F) as u8;
        self.polygon_id = (value >> 24 & 0x3F) as u8;
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, PartialEq)]
pub enum PolygonMode {
    Modulation = 0,
    Decal = 1,
    ToonHighlight = 2,
    Shadow = 3,
}

impl From<u32> for PolygonMode {
    fn from(value: u32) -> Self {
        match value {
            0 => PolygonMode::Modulation,
            1 => PolygonMode::Decal,
            2 => PolygonMode::ToonHighlight,
            3 => PolygonMode::Shadow,
            _ => unreachable!(),
        }
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
pub struct FrameParams {
    pub manual_sort_translucent: bool,
    pub w_buffer: bool,
}

impl FrameParams {
    pub fn new() -> Self {
        FrameParams { manual_sort_translucent: false, w_buffer: false }
    }

    pub fn write(&mut self, value: u32) {
        self.manual_sort_translucent = value & 0x1 != 0;
        self.w_buffer = (value >> 1) & 0x1 != 0;
    }
}

#[derive(emu_utils::Savestate)]
pub struct Viewport {
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    width: i32,
    height: i32,
}

impl Viewport {
    pub fn new() -> Self {
        Viewport { x1: 0, y1: 0, x2: 0, y2: 0, width: 0, height: 0 }
    }

    /// GBATEK "DS 3D Viewport" (`#ds3dviewsvolumesandviewports`): X/Y1/X/Y2
    /// are plain 8-bit coordinates (X: 0..255, Y: 0..191); hardware does not
    /// halt on values that would make width/height fall outside the screen,
    /// so out-of-range coordinates are clamped rather than asserted.
    pub fn write(&mut self, value: u32) {
        self.x1 = value as u8 as i32;
        self.y1 = ((value >> 8) as u8 as i32).min(GPU::HEIGHT as i32 - 1);
        self.x2 = (value >> 16) as u8 as i32;
        self.y2 = ((value >> 24) as u8 as i32).min(GPU::HEIGHT as i32 - 1);
        self.width = (self.x2 - self.x1 + 1).clamp(1, GPU::WIDTH as i32);
        self.height = (self.y2 - self.y1 + 1).clamp(1, GPU::HEIGHT as i32);
    }

    /// Projects a clip-space vertex to screen coordinates.
    ///
    /// GBATEK "DS 3D Viewport" (`#ds3dviewsvolumesandviewports`):
    /// SCREENX = (X+W)*(X2-X1+1)/(2W) + X1,
    /// SCREENY = (Y+W)*(Y2-Y1+1)/(2W) + Y1.
    ///
    /// The intermediate products are evaluated in `i64`. `X+W` spans up to
    /// `2W`, and W is a 1+19+12 fixed-point value, so `(X+W) * width` can
    /// exceed `i32` for large or distant geometry. In release builds that
    /// wraps to a negative value, which the final clamp then collapses onto
    /// the screen's left/top border: several vertices of one polygon snap to
    /// x=0, destroying its shape and making the rasterizer walk a span
    /// across most of the scanline. That is the direct cause of the
    /// full-width horizontal streaks documented in
    /// `docs/design/3d-background-rendering-design.md`.
    ///
    /// Because the polygon has already been clipped to the view volume
    /// (`-W <= X,Y,Z <= W`), a correctly computed result is always inside
    /// the viewport; the final clamp is therefore only a safety net against
    /// boundary rounding, never a substitute for clipping.
    pub fn screen_coords(&self, clip_coords: &Vec4) -> [u32; 2] {
        let w = clip_coords[3].raw() as i64;
        if w == 0 {
            [0, 0]
        } else {
            let x_offset = clip_coords[0].raw() as i64 + w;
            let y_offset = -(clip_coords[1].raw() as i64) + w;
            let denom = 2 * w;
            let x = x_offset * self.width as i64 / denom + self.x1 as i64;
            let y = y_offset * self.height as i64 / denom + self.y1 as i64;
            [x.clamp(0, GPU::WIDTH as i64 - 1) as u32, y.clamp(0, GPU::HEIGHT as i64 - 1) as u32]
        }
    }
}

#[derive(emu_utils::Savestate)]
pub enum VertexPrimitive {
    Triangles = 0,
    Quad = 1,
    TriangleStrips = 2,
    QuadStrips = 3,
}

impl From<u32> for VertexPrimitive {
    fn from(value: u32) -> Self {
        match value {
            0 => VertexPrimitive::Triangles,
            1 => VertexPrimitive::Quad,
            2 => VertexPrimitive::TriangleStrips,
            3 => VertexPrimitive::QuadStrips,
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::gpu::engine3d::math::FixedPoint;

    /// Full-screen viewport, as written by `VIEWPORT` = `0xBFFF0000`
    /// (X1=0, Y1=0, X2=255, Y2=191).
    fn full_screen() -> Viewport {
        let mut viewport = Viewport::new();
        viewport.write(0xBF_FF_00_00);
        viewport
    }

    fn project(viewport: &Viewport, x: i32, y: i32, w: i32) -> [u32; 2] {
        viewport.screen_coords(&Vec4::new(
            FixedPoint::from_frac12(x),
            FixedPoint::from_frac12(y),
            FixedPoint::from_frac12(0),
            FixedPoint::from_frac12(w),
        ))
    }

    #[test]
    fn projects_view_volume_corners_to_screen_corners() {
        let viewport = full_screen();
        let w = 0x1000;
        // X = -W maps to the left edge, X = +W to the right edge; Y is
        // flipped because screen Y grows downwards.
        assert_eq!(project(&viewport, -w, w, w), [0, 0]);
        assert_eq!(project(&viewport, 0, 0, w)[0], 128);
    }

    /// Regression test for the horizontal-streak / black-background bug.
    ///
    /// With a large W the intermediate `(X + W) * width` exceeds `i32`.
    /// When that product wrapped, the result went negative and the final
    /// clamp collapsed the vertex onto x=0, so several vertices of one
    /// polygon snapped to the screen edge and the rasterizer drew a span
    /// across most of the scanline. A vertex on the left half of the view
    /// volume must never project to the far right, and one on the right
    /// half must never project to x=0, no matter how large W is.
    #[test]
    fn large_w_does_not_overflow_the_projection() {
        let viewport = full_screen();
        for shift in 12..27 {
            let w = 1 << shift;
            assert_eq!(project(&viewport, -w, w, w), [0, 0], "left/top corner at w=2^{shift}");
            assert_eq!(
                project(&viewport, w, -w, w),
                [255, 191],
                "right/bottom corner at w=2^{shift}"
            );
            // The centre of the view volume stays at the centre of the screen.
            assert_eq!(project(&viewport, 0, 0, w), [128, 96], "centre at w=2^{shift}");
        }
    }
}
