use mem_const::*;

/// Display Control Register (DISPCNT)
///
/// Controls the main display settings, background visibility,
/// object rendering, and display modes.
#[derive(Debug, Default, Clone, Copy)]
pub struct DispCnt {
    /// Background mode (0-5)
    pub bg_mode: i32,
    /// Enable 3D mode
    pub bg_3d: bool,
    /// Use 1D mapping for character tiles for objects
    pub tile_obj_1d: bool,
    /// Use square bitmap objects
    pub bitmap_obj_square: bool,
    /// Use 1D mapping for bitmap objects
    pub bitmap_obj_1d: bool,
    /// Display background 0
    pub display_bg0: bool,
    /// Display background 1
    pub display_bg1: bool,
    /// Display background 2
    pub display_bg2: bool,
    /// Display background 3
    pub display_bg3: bool,
    /// Display objects (sprites)
    pub display_obj: bool,
    /// Display window 0
    pub display_win0: bool,
    /// Display window 1
    pub display_win1: bool,
    /// Display object window
    pub obj_win_display: bool,
    /// Display mode for layers
    pub display_mode: i32,
    /// VRAM block selection
    pub vram_block: i32,
    /// Tile object 1D boundary
    pub tile_obj_1d_bound: i32,
    /// Bitmap object 1D boundary
    pub bitmap_obj_1d_bound: bool,
    /// HBlank object processing enable
    pub hblank_obj_processing: bool,
    /// Character base block
    pub char_base: i32,
    /// Screen base block
    ///FIXME?: u32?(C++ impl is int)
    pub screen_base: i32,
    /// Enable extended palette for backgrounds
    pub bg_extended_palette: bool,
    /// Enable extended palette for objects
    pub obj_extended_palette: bool,
}

/// Display Capture Control Register (DISPCAPCNT)
///
/// Controls the capture of display content into VRAM.
#[derive(Debug, Default, Clone, Copy)]
pub struct DispCapCnt {
    /// Capture destination: EV_A
    pub eva: u8,
    /// Capture destination: EV_B
    pub evb: u8,
    /// VRAM write block selection
    pub vram_write_block: u8,
    /// VRAM write offset
    pub vram_write_offset: u8,
    /// Capture size (e.g., 128x128)
    pub capture_size: u8,
    /// Capture only from 3D engine A
    pub a_3d_only: bool,
    /// Display FIFO B during capture
    pub b_display_fifo: bool,
    /// VRAM read offset
    pub vram_read_offset: u8,
    /// Capture source selection
    pub capture_source: u8,
    /// Enable busy flag during capture
    pub enable_busy: bool,
}

/// Window Input Register (WININ)
///
/// Determines which layers and objects are visible inside windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinIn {
    /// Backgrounds enabled for window 0 (BG0-BG3)
    pub win0_bg_enabled: [bool; 4],
    /// Objects enabled for window 0
    pub win0_obj_enabled: bool,
    /// Special color effects for window 0
    pub win0_color_special: bool,
    /// Backgrounds enabled for window 1 (BG0-BG3)
    pub win1_bg_enabled: [bool; 4],
    /// Objects enabled for window 1
    pub win1_obj_enabled: bool,
    /// Special color effects for window 1
    pub win1_color_special: bool,
}

/// Window Output Register (WINOUT)
///
/// Determines which layers and objects are visible outside windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinOut {
    /// Backgrounds enabled outside windows (BG0-BG3)
    pub outside_bg_enabled: [bool; 4],
    /// Objects enabled outside windows
    pub outside_obj_enabled: bool,
    /// Special color effects outside windows
    pub outside_color_special: bool,
    /// Backgrounds enabled in object window (BG0-BG3)
    pub objwin_bg_enabled: [bool; 4],
    /// Objects enabled in object window
    pub objwin_obj_enabled: bool,
    /// Special color effects in object window
    pub objwin_color_special: bool,
}

/// Blend Control Register (BLDCNT)
///
/// Controls alpha blending, brightness increase/decrease, and
/// which layers participate in the blend.
#[derive(Debug, Default, Clone, Copy)]
pub struct BldCnt {
    /// Backgrounds as first target pixels (BG0-BG3)
    pub bg_first_target_pix: [bool; 4],
    /// Objects as first target pixels
    pub obj_first_target_pix: bool,
    /// Backdrop as first target pixel
    pub bd_first_target_pix: bool,
    /// Effect type (0=none, 1=alpha blend, 2=lighten, 3=darken)
    pub effect: u8,
    /// Backgrounds as second target pixels (BG0-BG3)
    pub bg_second_target_pix: [bool; 4],
    /// Objects as second target pixels
    pub obj_second_target_pix: bool,
    /// Backdrop as second target pixel
    pub bd_second_target_pix: bool,
}

pub struct Gpu2DEngine {
    // Frame buffers
    /// size: 49152(PIXELS_PER_LINE * SCAN_LINES)
    pub framebuffer: Vec<u32>,
    /// size: 49152(PIXELS_PER_LINE * SCAN_LINES)
    pub front_framebuffer: Vec<u32>,

    // size: 512(PIXELS_PER_LINE * 2)
    pub final_bg_priority: Vec<u8>,
    // size: 512(PIXELS_PER_LINE * 2)
    pub sprite_scanline: Vec<u32>,
    // Scanline buffers 256(PIXELS_PER_LINE)
    pub window_mask: Vec<u8>,

    // Registers
    pub dispcnt: DispCnt,
    pub dispcapcnt: DispCapCnt,
    pub captured_lines: i32,

    pub bgcnt: [u16; 4],
    pub bghofs: [u16; 4],
    pub bgvofs: [u16; 4],

    pub bg2p: [u16; 4],
    pub bg3p: [u16; 4],

    pub bg2x: u32,
    pub bg2y: u32,
    pub bg3x: u32,
    pub bg3y: u32,

    pub bg2p_internal: [u16; 4],
    pub bg3p_internal: [u16; 4],

    pub bg2x_internal: i32,
    pub bg2y_internal: i32,
    pub bg3x_internal: i32,
    pub bg3y_internal: i32,

    pub win0h: u16,
    pub win1h: u16,
    pub win0v: u16,
    pub win1v: u16,

    pub mosaic: u16,

    pub winin: WinIn,
    pub winout: WinOut,
    pub win0_active: bool,
    pub win1_active: bool,

    pub bldcnt: BldCnt,
    pub bldalpha: u16,
    pub bldy: u8,

    pub master_bright: u16,
}

impl Gpu2DEngine {
    pub(crate) fn new() -> Self {
        Gpu2DEngine {
            framebuffer: vec![0; PIXELS_PER_LINE * SCANLINES],
            front_framebuffer: vec![0; PIXELS_PER_LINE * SCANLINES],

            final_bg_priority: vec![0; PIXELS_PER_LINE * 2],
            sprite_scanline: vec![0; PIXELS_PER_LINE * 2],
            window_mask: vec![0; PIXELS_PER_LINE],

            dispcnt: DispCnt::default(),
            dispcapcnt: DispCapCnt::default(),
            captured_lines: 0,

            bgcnt: [0; 4],
            bghofs: [0; 4],
            bgvofs: [0; 4],

            bg2p: [0; 4],
            bg3p: [0; 4],
            bg2x: 0,
            bg2y: 0,
            bg3x: 0,
            bg3y: 0,

            bg2p_internal: [0; 4],
            bg3p_internal: [0; 4],

            bg2x_internal: 0,
            bg2y_internal: 0,
            bg3x_internal: 0,
            bg3y_internal: 0,

            win0h: 0,
            win1h: 0,
            win0v: 0,
            win1v: 0,

            mosaic: 0,

            winin: WinIn::default(),
            winout: WinOut::default(),
            win0_active: false,
            win1_active: false,

            bldcnt: BldCnt::default(),
            bldalpha: 0,
            bldy: 0,

            master_bright: 0,
        }
    }

    // ============================================================
    // Framebuffer handling
    // ============================================================

    /// Copies the internal framebuffer into `buffer`.
    pub fn get_framebuffer(&self, buffer: &mut [u32]) {
        // Ensure the buffer is large enough
        assert!(buffer.len() >= SCANLINES * PIXELS_PER_LINE);

        for y in 0..SCANLINES {
            let line_start = y * PIXELS_PER_LINE;
            for x in 0..PIXELS_PER_LINE {
                buffer[line_start + x] = self.front_framebuffer[line_start + x];
            }
        }

        // Debug output (commented out)
        /*
        if self.engine_a {
            for i in 0..5 {
                println!("Engine A final color data: ${:08X}", buffer[i]);
            }
        }
        */
    }

    /// Sets an external framebuffer as the render target.
    /// C++ code is empty.
    pub fn set_framebuffer(&mut self, buffer: &mut [u32]) {
        // framebuffer = buffer;
    }

    // ============================================================
    // Timing
    // ============================================================

    /// Called at the start of VBLANK.
    pub fn vblank_start(&mut self) {
        todo!()
    }

    // ============================================================
    // Register getters
    // ============================================================

    /// Returns the display control register (DISPCNT).
    pub fn get_dispcnt(&self) -> u32 {
        let mut reg: u32 = 0;

        reg |= self.dispcnt.bg_mode;
        reg |= (self.dispcnt.bg_3d as u32) << 3;
        reg |= (self.dispcnt.tile_obj_1d as u32) << 4;
        reg |= (self.dispcnt.bitmap_obj_square as u32) << 5;
        reg |= (self.dispcnt.bitmap_obj_1d as u32) << 6;
        reg |= (self.dispcnt.display_bg0 as u32) << 8;
        reg |= (self.dispcnt.display_bg1 as u32) << 9;
        reg |= (self.dispcnt.display_bg2 as u32) << 10;
        reg |= (self.dispcnt.display_bg3 as u32) << 11;
        reg |= (self.dispcnt.display_obj as u32) << 12;
        reg |= (self.dispcnt.display_win0 as u32) << 13;
        reg |= (self.dispcnt.display_win1 as u32) << 14;
        reg |= (self.dispcnt.obj_win_display as u32) << 15;
        reg |= (self.dispcnt.display_mode as u32) << 16;
        reg |= (self.dispcnt.vram_block as u32) << 18;
        reg |= (self.dispcnt.tile_obj_1d_bound as u32) << 20;
        reg |= (self.dispcnt.bitmap_obj_1d_bound as u32) << 22;
        reg |= (self.dispcnt.hblank_obj_processing as u32) << 23;
        reg |= (self.dispcnt.char_base as u32) << 24;
        reg |= (self.dispcnt.screen_base as u32) << 27;
        reg |= (self.dispcnt.bg_extended_palette as u32) << 30;
        reg |= (self.dispcnt.obj_extended_palette as u32) << 31;

        reg
    }

    /// Returns a background control register (BGCNT).
    pub fn get_bgcnt(&self, index: usize) -> u16 {
        self.bgcnt[index]
    }

    pub fn get_bghofs(&self, index: usize) -> u16 {
        self.bghofs[index]
    }

    pub fn get_bgvofs(&self, index: usize) -> u16 {
        self.bgvofs[index]
    }

    pub fn get_win0v(&self) -> u16 {
        self.win0v
    }

    pub fn get_win1v(&self) -> u16 {
        self.win1v
    }

    pub fn get_winin(&self) -> u16 {
        let mut reg: u16 = 0;

        for bit in 0..4 {
            reg |= (self.winin.win0_bg_enabled[bit] as u16) << bit;
            reg |= (self.winin.win1_bg_enabled[bit] as u16) << (bit + 8);
        }

        reg |= (self.winin.win0_obj_enabled as u16) << 4;
        reg |= (self.winin.win0_color_special as u16) << 5;
        reg |= (self.winin.win1_obj_enabled as u16) << 12;
        reg |= (self.winin.win1_color_special as u16) << 13;

        reg
    }

    pub fn get_winout(&self) -> u16 {
        let mut reg: u16 = 0;

        for bit in 0..4 {
            reg |= (self.winout.outside_bg_enabled[bit] as u16) << bit;
            reg |= (self.winout.objwin_bg_enabled[bit] as u16) << (bit + 8);
        }

        reg |= (self.winout.outside_obj_enabled as u16) << 4;
        reg |= (self.winout.outside_color_special as u16) << 5;
        reg |= (self.winout.objwin_obj_enabled as u16) << 12;
        reg |= (self.winout.objwin_color_special as u16) << 13;

        reg
    }

    pub fn get_bldcnt(&self) -> u16 {
        let mut reg: u16 = 0;

        for bit in 0..4 {
            reg |= (self.bldcnt.bg_first_target_pix[bit] as u16) << bit;
            reg |= (self.bldcnt.bg_second_target_pix[bit] as u16) << (bit + 8);
        }

        reg |= (self.bldcnt.obj_first_target_pix as u16) << 4;
        reg |= (self.bldcnt.obj_second_target_pix as u16) << 12;
        reg |= (self.bldcnt.bd_first_target_pix as u16) << 5;
        reg |= (self.bldcnt.bd_second_target_pix as u16) << 13;
        reg |= (self.bldcnt.effect as u16) << 6;

        reg
    }

    pub fn get_bldalpha(&self) -> u16 {
        self.bldalpha
    }

    pub fn get_master_bright(&self) -> u16 {
        self.master_bright
    }

    pub fn get_dispcapcnt(&self) -> u32 {
        if !self.engine_a {
            return 0;
        }

        let mut reg: u32 = 0;
        reg |= self.dispcapcnt.eva as u32;
        reg |= (self.dispcapcnt.evb as u32) << 8;
        reg |= (self.dispcapcnt.vram_write_block as u32) << 16;
        reg |= (self.dispcapcnt.vram_write_offset as u32) << 18;
        reg |= (self.dispcapcnt.capture_size as u32) << 20;
        reg |= (self.dispcapcnt.a_3d_only as u32) << 24;
        reg |= (self.dispcapcnt.b_display_fifo as u32) << 25;
        reg |= (self.dispcapcnt.vram_read_offset as u32) << 26;
        reg |= (self.dispcapcnt.capture_source as u32) << 29;
        reg |= (self.dispcapcnt.enable_busy as u32) << 31;

        reg
    }

    // ============================================================
    // Register setters
    // ============================================================

    /// Sets the low 16 bits of DISPCNT.
    pub fn set_disp_cnt_lo(&mut self, halfword: u16) {
        self.dispcnt.bg_mode = halfword & 0x7;
        self.dispcnt.bg_3d = (halfword & (1 << 3)) != 0;
        self.dispcnt.tile_obj_1d = (halfword & (1 << 4)) != 0;
        self.dispcnt.bitmap_obj_square = (halfword & (1 << 5)) != 0;
        self.dispcnt.bitmap_obj_1d = (halfword & (1 << 6)) != 0;
        self.dispcnt.display_bg0 = (halfword & (1 << 8)) != 0;
        self.dispcnt.display_bg1 = (halfword & (1 << 9)) != 0;
        self.dispcnt.display_bg2 = (halfword & (1 << 10)) != 0;
        self.dispcnt.display_bg3 = (halfword & (1 << 11)) != 0;
        self.dispcnt.display_obj = (halfword & (1 << 12)) != 0;
        self.dispcnt.display_win0 = (halfword & (1 << 13)) != 0;
        self.dispcnt.display_win1 = (halfword & (1 << 14)) != 0;
        self.dispcnt.obj_win_display = (halfword & (1 << 15)) != 0;
    }

    /// Sets the full 32-bit DISPCNT.
    pub fn set_disp_cnt(&mut self, word: u32) {
        self.set_disp_cnt_lo((word & 0xFFFF) as u16);

        self.dispcnt.display_mode = ((word >> 16) & 0x3) as u8;
        self.dispcnt.vram_block = ((word >> 18) & 0x3) as u8;
        self.dispcnt.tile_obj_1d_bound = ((word >> 20) & 0x3) as u8;
        self.dispcnt.bitmap_obj_1d_bound = (word & (1 << 22)) != 0;
        self.dispcnt.hblank_obj_processing = (word & (1 << 23)) != 0;
        self.dispcnt.char_base = ((word >> 24) & 0x7) as u8;
        self.dispcnt.screen_base = ((word >> 27) & 0x7) as u8;
        self.dispcnt.bg_extended_palette = (word & (1 << 30)) != 0;
        self.dispcnt.obj_extended_palette = (word & (1 << 31)) != 0;
    }

    pub fn set_bgcnt(&mut self, halfword: u16, index: usize) {
        self.bgcnt[index] = halfword;
    }

    pub fn set_bghofs(&mut self, halfword: u16, index: usize) {
        self.bghofs[index] = halfword;
    }

    pub fn set_bgvofs(&mut self, halfword: u16, index: usize) {
        self.bgvofs[index] = halfword;
    }

    /// Sets BG2/BG3 affine parameter (PA, PB, PC, PD).
    pub fn set_bg2p(&mut self, halfword: u16, index: usize) {
        self.bg2p[index] = halfword;
        if self.gpu.get_vcount() < 192 {
            self.bg2p_internal[index] = halfword;
        }
    }

    pub fn set_bg3p(&mut self, halfword: u16, index: usize) {
        self.bg3p[index] = halfword;
        if self.gpu.get_vcount() < 192 {
            self.bg3p_internal[index] = halfword;
        }
    }

    pub fn set_bg2x(&mut self, word: u32) {
        self.bg2x = word;
        if self.gpu.get_vcount() < 192 {
            self.bg2x_internal = word;
        }
    }

    pub fn set_bg2y(&mut self, word: u32) {
        self.bg2y = word;
        if self.gpu.get_vcount() < 192 {
            self.bg2y_internal = word;
        }
    }

    pub fn set_bg3x(&mut self, word: u32) {
        self.bg3x = word;
        if self.gpu.get_vcount() < 192 {
            self.bg3x_internal = word;
        }
    }

    pub fn set_bg3y(&mut self, word: u32) {
        self.bg3y = word;
        if self.gpu.get_vcount() < 192 {
            self.bg3y_internal = word;
        }
    }

    pub fn set_winin(&mut self, halfword: u16) {
        for bit in 0..4 {
            self.winin.win0_bg_enabled[bit] = (halfword & (1 << bit)) != 0;
            self.winin.win1_bg_enabled[bit] = (halfword & (1 << (bit + 8))) != 0;
        }
        self.winin.win0_obj_enabled = (halfword & (1 << 4)) != 0;
        self.winin.win0_color_special = (halfword & (1 << 5)) != 0;
        self.winin.win1_obj_enabled = (halfword & (1 << 12)) != 0;
        self.winin.win1_color_special = (halfword & (1 << 13)) != 0;
    }

    pub fn set_winout(&mut self, halfword: u16) {
        for bit in 0..4 {
            self.winout.outside_bg_enabled[bit] = (halfword & (1 << bit)) != 0;
            self.winout.objwin_bg_enabled[bit] = (halfword & (1 << (bit + 8))) != 0;
        }
        self.winout.outside_obj_enabled = (halfword & (1 << 4)) != 0;
        self.winout.outside_color_special = (halfword & (1 << 5)) != 0;
        self.winout.objwin_obj_enabled = (halfword & (1 << 12)) != 0;
        self.winout.objwin_color_special = (halfword & (1 << 13)) != 0;
    }

    pub fn set_mosaic(&mut self, halfword: u16) {
        self.mosaic = halfword;
    }

    pub fn set_bldcnt(&mut self, halfword: u16) {
        for bit in 0..4 {
            self.bldcnt.bg_first_target_pix[bit] = (halfword & (1 << bit)) != 0;
            self.bldcnt.bg_second_target_pix[bit] = (halfword & (1 << (bit + 8))) != 0;
        }
        self.bldcnt.obj_first_target_pix = (halfword & (1 << 4)) != 0;
        self.bldcnt.obj_second_target_pix = (halfword & (1 << 12)) != 0;

        self.bldcnt.bd_first_target_pix = (halfword & (1 << 5)) != 0;
        self.bldcnt.bd_second_target_pix = (halfword & (1 << 13)) != 0;

        self.bldcnt.effect = ((halfword >> 6) & 0x3) as u8;
    }

    pub fn set_bldalpha(&mut self, halfword: u16) {
        self.bldalpha = halfword;
    }

    pub fn set_bldy(&mut self, byte: u8) {
        self.bldy = byte;
    }

    pub fn set_master_bright(&mut self, halfword: u16) {
        self.master_bright = halfword;
    }

    pub fn set_dispcapcnt(&mut self, word: u32) {
        // if (!engine_A) return;
        if !self.engine_a {
            return;
        }

        // EVA (0..=16 clamp)
        let eva = (word & 0x1F) as u8;
        self.dispcapcnt.eva = eva.min(16);

        // EVB (0..=16 clamp)
        let evb = ((word >> 8) & 0x1F) as u8;
        self.dispcapcnt.evb = evb.min(16);

        // VRAM write block (2 bits)
        self.dispcapcnt.vram_write_block = ((word >> 16) & 0x3) as u8;

        // VRAM write offset (2 bits)
        self.dispcapcnt.vram_write_offset = ((word >> 18) & 0x3) as u8;

        // Capture size (2 bits)
        self.dispcapcnt.capture_size = ((word >> 20) & 0x3) as u8;

        // Flags
        self.dispcapcnt.a_3d_only = (word & (1 << 24)) != 0;

        self.dispcapcnt.b_display_fifo = (word & (1 << 25)) != 0;

        // VRAM read offset (2 bits)
        self.dispcapcnt.vram_read_offset = ((word >> 26) & 0x3) as u8;

        // Capture source (2 bits)
        self.dispcapcnt.capture_source = ((word >> 29) & 0x3) as u8;

        // Rising edge of enable → reset captured_lines
        let enable = (word & (1 << 31)) != 0;
        if !self.dispcapcnt.enable_busy && enable {
            self.captured_lines = -1;
        }

        self.dispcapcnt.enable_busy = enable;
    }
}
