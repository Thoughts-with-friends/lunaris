//! The two 2D engines (A: main, may show 3D as BG0; B: sub). Each scanline
//! renders BGs ([`bg`]) and OBJs ([`obj`]) into line buffers that
//! [`compose`] combines with windows ([`window`]) and color effects.
//!
//! ## How a 2D engine builds a picture
//!
//! The screen is 256x192 pixels and is drawn one scanline (row) at a time.
//! Each line is built from up to four background layers (BG0-BG3) and the
//! sprite layer (OBJ), stacked by priority in front of a backdrop color:
//!
//! ```text
//!   viewer
//!     |     priority 0  (front)   BG pixels (BGxCNT bits 0-1) and
//!     v     priority 1            OBJ pixels (OAM attr2 bits 10-11)
//!           priority 2
//!           priority 3  (back)
//!           backdrop              BG palette color 0
//!
//!   same priority: OBJ is in front of BG, and BG0 > BG1 > BG2 > BG3
//!
//!   one scanline (y = VCOUNT), every layer rendered into its own line buffer:
//!   x=0                                   x=255
//!   +---------------------------------------+
//!   | BG0   . . . a a a . . . . . . . . . . |   '.' = transparent pixel
//!   | BG1   b b b b b b b b b b b b b b b b |
//!   | OBJ   . . . . . o o o . . . . . . . . |
//!   +---------------------------------------+
//!             |  per pixel: first opaque layer in priority order
//!             v  (all three have the same priority here)
//!   result  b b b a a o o o b b b b b b b b b
//! ```
//!
//! Which kind of layer each BG is depends on the BG mode (DISPCNT bits 0-2):
//!
//! ```text
//!   mode   BG0        BG1    BG2        BG3
//!   0      text/3D    text   text       text
//!   1      text/3D    text   text       affine
//!   2      text/3D    text   affine     affine
//!   3      text/3D    text   text       extended
//!   4      text/3D    text   affine     extended
//!   5      text/3D    text   extended   extended
//!   6      text/3D    -      large      -          (engine A only)
//!
//!   "3D": engine A shows the 3D engine's image as BG0 when DISPCNT bit 3 is set.
//! ```
//!
//! See `bg.rs` for the layer types, `obj.rs` for sprites, `window.rs` and
//! `compose.rs` for how the layers are combined.
//! GBATEK: [DS Video](https://problemkaputt.de/gbatek.htm#dsvideo),
//! [DS Video BG Modes / Control](https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol)
mod bg;
pub(crate) mod compose;
mod obj;
pub mod regs;
mod window;

use ds_mem_addr::*;
pub use regs::{BldCnt, DispCapCnt, DispCnt, WinIn, WinOut};

#[derive(serde::Serialize, serde::Deserialize, Debug, Default)]
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
    /// Creates an engine with all registers cleared.
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

    /// Copies the internal framebuffer into `buffer` (from
    /// Gpu::get_upper_frame / get_lower_frame).
    pub fn get_framebuffer(&self, buffer: &mut [u32]) {
        // Ensure the buffer is large enough
        assert!(buffer.len() >= SCANLINES * PIXELS_PER_LINE);

        for y in 0..SCANLINES {
            let line_start = y * PIXELS_PER_LINE;
            let line_end = line_start + PIXELS_PER_LINE;

            buffer[line_start..line_end]
                .copy_from_slice(&self.front_framebuffer[line_start..line_end]);
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
    pub fn set_framebuffer(&mut self, buffer: Vec<u32>) {
        let _framebuffer = buffer;
        unimplemented!("C++ code is empty.")
    }

    // ============================================================
    // Timing
    // ============================================================

    /// Called at the start of VBLANK (ds_emu gpu_handle_event, line 192):
    /// reloads the BG2/BG3 affine reference points and ends a display
    /// capture.
    pub fn vblank_start(&mut self) {
        for i in 0..4 {
            self.bg2p_internal[i] = self.bg2p[i];
            self.bg3p_internal[i] = self.bg3p[i];
        }

        self.bg2x_internal = self.bg2x as i32;
        self.bg2y_internal = self.bg2y as i32;
        self.bg3x_internal = self.bg3x as i32;
        self.bg3y_internal = self.bg3y as i32;

        self.dispcapcnt.enable_busy = false;
    }

    // ============================================================
    // Register getters
    // ============================================================

    /// Returns the display control register (DISPCNT).
    pub fn get_dispcnt(&self) -> u32 {
        let mut reg: u32 = 0;

        reg |= self.dispcnt.bg_mode as u32;
        // LCD I/O Display Control: https://problemkaputt.de/gbatek.htm#lcdiodisplaycontrol
        reg |= (self.dispcnt.bg_3d as u32) << 3; // DISPCNT bit 3: bg 3d
        reg |= (self.dispcnt.tile_obj_1d as u32) << 4; // DISPCNT bit 4: tile obj 1d
        reg |= (self.dispcnt.bitmap_obj_square as u32) << 5; // DISPCNT bit 5: bitmap obj square
        reg |= (self.dispcnt.bitmap_obj_1d as u32) << 6; // DISPCNT bit 6: bitmap obj 1d
        reg |= (self.dispcnt.display_bg0 as u32) << 8; // DISPCNT bit 8: display bg0
        reg |= (self.dispcnt.display_bg1 as u32) << 9; // DISPCNT bit 9: display bg1
        reg |= (self.dispcnt.display_bg2 as u32) << 10; // DISPCNT bit 10: display bg2
        reg |= (self.dispcnt.display_bg3 as u32) << 11; // DISPCNT bit 11: display bg3
        reg |= (self.dispcnt.display_obj as u32) << 12; // DISPCNT bit 12: display obj
        reg |= (self.dispcnt.display_win0 as u32) << 13; // DISPCNT bit 13: display win0
        reg |= (self.dispcnt.display_win1 as u32) << 14; // DISPCNT bit 14: display win1
        reg |= (self.dispcnt.obj_win_display as u32) << 15; // DISPCNT bit 15: obj win display
        reg |= (self.dispcnt.display_mode as u32) << 16; // DISPCNT bit 16: display mode
        reg |= (self.dispcnt.vram_block as u32) << 18; // DISPCNT bit 18: vram block
        reg |= (self.dispcnt.tile_obj_1d_bound as u32) << 20; // DISPCNT bit 20: tile obj 1d bound
        reg |= (self.dispcnt.bitmap_obj_1d_bound as u32) << 22; // DISPCNT bit 22: bitmap obj 1d bound
        reg |= (self.dispcnt.hblank_obj_processing as u32) << 23; // DISPCNT bit 23: hblank obj processing
        reg |= (self.dispcnt.char_base as u32) << 24; // DISPCNT bit 24: char base
        reg |= (self.dispcnt.screen_base as u32) << 27; // DISPCNT bit 27: screen base
        reg |= (self.dispcnt.bg_extended_palette as u32) << 30; // DISPCNT bit 30: bg extended palette
        reg |= (self.dispcnt.obj_extended_palette as u32) << 31; // DISPCNT bit 31: obj extended palette

        reg
    }

    /// Returns a background control register (BGCNT).
    pub fn get_bgcnt(&self, index: usize) -> u16 {
        self.bgcnt[index]
    }

    /// Raw BGxHOFS value.
    pub fn get_bghofs(&self, index: usize) -> u16 {
        self.bghofs[index]
    }

    /// Raw BGxVOFS value.
    pub fn get_bgvofs(&self, index: usize) -> u16 {
        self.bgvofs[index]
    }

    /// Raw WIN0V value.
    pub fn get_win0v(&self) -> u16 {
        self.win0v
    }

    /// Raw WIN1V value.
    pub fn get_win1v(&self) -> u16 {
        self.win1v
    }

    /// WININ packed from the decoded window flags.
    pub fn get_winin(&self) -> u16 {
        let mut reg: u16 = 0;

        for bit in 0..4 {
            reg |= (self.winin.win0_bg_enabled[bit] as u16) << bit;
            reg |= (self.winin.win1_bg_enabled[bit] as u16) << (bit + 8);
        }

        // LCD I/O Window Feature: https://problemkaputt.de/gbatek.htm#lcdiowindowfeature
        reg |= (self.winin.win0_obj_enabled as u16) << 4; // WININ bit 4: win0 obj enabled
        reg |= (self.winin.win0_color_special as u16) << 5; // WININ bit 5: win0 color special
        reg |= (self.winin.win1_obj_enabled as u16) << 12; // WININ bit 12: win1 obj enabled
        reg |= (self.winin.win1_color_special as u16) << 13; // WININ bit 13: win1 color special

        reg
    }

    /// WINOUT packed from the decoded window flags.
    pub fn get_winout(&self) -> u16 {
        let mut reg: u16 = 0;

        for bit in 0..4 {
            reg |= (self.winout.outside_bg_enabled[bit] as u16) << bit;
            reg |= (self.winout.objwin_bg_enabled[bit] as u16) << (bit + 8);
        }

        // LCD I/O Window Feature: https://problemkaputt.de/gbatek.htm#lcdiowindowfeature
        reg |= (self.winout.outside_obj_enabled as u16) << 4; // WINOUT bit 4: outside obj enabled
        reg |= (self.winout.outside_color_special as u16) << 5; // WINOUT bit 5: outside color special
        reg |= (self.winout.objwin_obj_enabled as u16) << 12; // WINOUT bit 12: objwin obj enabled
        reg |= (self.winout.objwin_color_special as u16) << 13; // WINOUT bit 13: objwin color special

        reg
    }

    /// BLDCNT packed from the decoded blend targets.
    pub fn get_bldcnt(&self) -> u16 {
        let mut reg: u16 = 0;

        for bit in 0..4 {
            reg |= (self.bldcnt.bg_first_target_pix[bit] as u16) << bit;
            reg |= (self.bldcnt.bg_second_target_pix[bit] as u16) << (bit + 8);
        }

        // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
        reg |= (self.bldcnt.obj_first_target_pix as u16) << 4; // BLDCNT bit 4: obj first target pix
        reg |= (self.bldcnt.obj_second_target_pix as u16) << 12; // BLDCNT bit 12: obj second target pix
        reg |= (self.bldcnt.bd_first_target_pix as u16) << 5; // BLDCNT bit 5: bd first target pix
        reg |= (self.bldcnt.bd_second_target_pix as u16) << 13; // BLDCNT bit 13: bd second target pix
        reg |= (self.bldcnt.effect as u16) << 6; // BLDCNT bit 6: effect

        reg
    }

    /// Raw BLDALPHA value.
    pub fn get_bldalpha(&self) -> u16 {
        self.bldalpha
    }

    /// Raw MASTER_BRIGHT value.
    pub fn get_master_bright(&self) -> u16 {
        self.master_bright
    }

    /// DISPCAPCNT packed (engine A only; 0 for engine B).
    pub fn get_dispcapcnt(&self, is_engine_a: bool) -> u32 {
        if !is_engine_a {
            return 0;
        }

        let mut reg: u32 = 0;
        reg |= self.dispcapcnt.eva as u32;
        // DS Video Capture and Main Memory Display Mode: https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode
        reg |= (self.dispcapcnt.evb as u32) << 8; // DISPCAPCNT bit 8: evb
        reg |= (self.dispcapcnt.vram_write_block as u32) << 16; // DISPCAPCNT bit 16: vram write block
        reg |= (self.dispcapcnt.vram_write_offset as u32) << 18; // DISPCAPCNT bit 18: vram write offset
        reg |= (self.dispcapcnt.capture_size as u32) << 20; // DISPCAPCNT bit 20: capture size
        reg |= (self.dispcapcnt.a_3d_only as u32) << 24; // DISPCAPCNT bit 24: a 3d only
        reg |= (self.dispcapcnt.b_display_fifo as u32) << 25; // DISPCAPCNT bit 25: b display fifo
        reg |= (self.dispcapcnt.vram_read_offset as u32) << 26; // DISPCAPCNT bit 26: vram read offset
        reg |= (self.dispcapcnt.capture_source as u32) << 29; // DISPCAPCNT bit 29: capture source
        reg |= (self.dispcapcnt.enable_busy as u32) << 31; // DISPCAPCNT bit 31: enable busy

        reg
    }

    // ============================================================
    // Register setters
    // ============================================================

    /// Sets the low 16 bits of DISPCNT.
    pub fn set_dispcnt_lo(&mut self, halfword: u16) {
        // LCD I/O Display Control: https://problemkaputt.de/gbatek.htm#lcdiodisplaycontrol
        self.dispcnt.bg_mode = (halfword & 0x7) as i32; // BG mode (bits 0-2)
        // LCD I/O Display Control: https://problemkaputt.de/gbatek.htm#lcdiodisplaycontrol
        self.dispcnt.bg_3d = (halfword & (1 << 3)) != 0; // DISPCNT bit 3: bg 3d
        self.dispcnt.tile_obj_1d = (halfword & (1 << 4)) != 0; // DISPCNT bit 4: tile obj 1d
        self.dispcnt.bitmap_obj_square = (halfword & (1 << 5)) != 0; // DISPCNT bit 5: bitmap obj square
        self.dispcnt.bitmap_obj_1d = (halfword & (1 << 6)) != 0; // DISPCNT bit 6: bitmap obj 1d
        self.dispcnt.display_bg0 = (halfword & (1 << 8)) != 0; // DISPCNT bit 8: display bg0
        self.dispcnt.display_bg1 = (halfword & (1 << 9)) != 0; // DISPCNT bit 9: display bg1
        self.dispcnt.display_bg2 = (halfword & (1 << 10)) != 0; // DISPCNT bit 10: display bg2
        self.dispcnt.display_bg3 = (halfword & (1 << 11)) != 0; // DISPCNT bit 11: display bg3
        self.dispcnt.display_obj = (halfword & (1 << 12)) != 0; // DISPCNT bit 12: display obj
        self.dispcnt.display_win0 = (halfword & (1 << 13)) != 0; // DISPCNT bit 13: display win0
        self.dispcnt.display_win1 = (halfword & (1 << 14)) != 0; // DISPCNT bit 14: display win1
        self.dispcnt.obj_win_display = (halfword & (1 << 15)) != 0; // DISPCNT bit 15: obj win display
    }

    /// Sets the full 32-bit DISPCNT.
    pub fn set_dispcnt(&mut self, word: u32) {
        self.set_dispcnt_lo((word & 0xFFFF) as u16); // lower halfword

        self.dispcnt.display_mode = ((word >> 16) & 0x3) as i32; // display mode (bits 16-17)
        self.dispcnt.vram_block = ((word >> 18) & 0x3) as i32; // VRAM block for display mode 2 (bits 18-19)
        self.dispcnt.tile_obj_1d_bound = ((word >> 20) & 0x3) as i32; // tile OBJ 1D boundary (bits 20-21)
        // LCD I/O Display Control: https://problemkaputt.de/gbatek.htm#lcdiodisplaycontrol
        self.dispcnt.bitmap_obj_1d_bound = (word & (1 << 22)) != 0; // DISPCNT bit 22: bitmap obj 1d bound
        self.dispcnt.hblank_obj_processing = (word & (1 << 23)) != 0; // DISPCNT bit 23: hblank obj processing
        self.dispcnt.char_base = ((word >> 24) & 0x7) as i32; // character base (bits 24-26)
        self.dispcnt.screen_base = ((word >> 27) & 0x7) as i32; // screen base (bits 27-29)
        self.dispcnt.bg_extended_palette = (word & (1 << 30)) != 0; // DISPCNT bit 30: bg extended palette
        self.dispcnt.obj_extended_palette = (word & (1 << 31)) != 0; // DISPCNT bit 31: obj extended palette

        log::trace!(
            target: "gpu_2d",
            "DISPCNT updated: 0x{:08X}\n\
            - display_mode: {}\n\
            - vram_block: {}\n\
            - tile_obj_1d_bound: {}\n\
            - bitmap_obj_1d_bound: {}\n\
            - hblank_obj: {}\n\
            - char_base: {}\n\
            - screen_base: {}\n\
            - bg_ext_pal: {}\n\
            - obj_ext_pal: {}",
            word,
            self.dispcnt.display_mode,
            self.dispcnt.vram_block,
            self.dispcnt.tile_obj_1d_bound,
            self.dispcnt.bitmap_obj_1d_bound,
            self.dispcnt.hblank_obj_processing,
            self.dispcnt.char_base,
            self.dispcnt.screen_base,
            self.dispcnt.bg_extended_palette,
            self.dispcnt.obj_extended_palette
        );
    }

    /// Decodes BGxCNT (priority, tile/map bases, size, palette mode).
    pub fn set_bgcnt(&mut self, halfword: u16, index: usize) {
        self.bgcnt[index] = halfword;

        // LCD I/O BG Control: https://problemkaputt.de/gbatek.htm#lcdiobgcontrol
        let priority = halfword & 0x3; // BG priority (bits 0-1)
        let char_base = (halfword >> 2) & 0xF; // character base block (bits 2-5)
        // LCD I/O BG Control: https://problemkaputt.de/gbatek.htm#lcdiobgcontrol
        let mosaic = (halfword & (1 << 6)) != 0; // BGxCNT bit 6
        let color_mode_256 = (halfword & (1 << 7)) != 0; // BGxCNT bit 7
        let screen_base = (halfword >> 8) & 0x1F; // screen base block (bits 8-12)
        let screen_size = (halfword >> 14) & 0x3; // screen size (bits 14-15)

        log::trace!(
            target: "gpu_bg",
            "[GPU BackGround] BG{}CNT Update: 0x{:04X} | Prio:{} CharBase:{} Mosaic: {} ScreenBase:{} 256Color:{} Size:{}",
            index, halfword, priority, char_base, mosaic, screen_base, color_mode_256, screen_size
        );
    }

    /// Stores BGxHOFS (9-bit X scroll).
    pub fn set_bghofs(&mut self, halfword: u16, index: usize) {
        self.bghofs[index] = halfword;
    }

    /// Stores BGxVOFS (9-bit Y scroll).
    pub fn set_bgvofs(&mut self, halfword: u16, index: usize) {
        self.bgvofs[index] = halfword;
    }

    /// Sets BG2/BG3 affine parameter (PA, PB, PC, PD).
    pub fn set_bg2p(&mut self, halfword: u16, index: usize, vcount: u16) {
        self.bg2p[index] = halfword;
        if vcount < 192 {
            self.bg2p_internal[index] = halfword;
        }
    }

    /// Sets BG3 affine parameter (PA, PB, PC, PD).
    pub fn set_bg3p(&mut self, halfword: u16, index: usize, vcount: u16) {
        self.bg3p[index] = halfword;
        if vcount < 192 {
            self.bg3p_internal[index] = halfword;
        }
    }

    /// Sets BG2X (28-bit reference X); on lines 0-191 the internal reference is reloaded too.
    pub fn set_bg2x(&mut self, word: u32, vcount: u16) {
        self.bg2x = word;
        if vcount < 192 {
            self.bg2x_internal = word as i32;
        }
    }

    /// Sets BG2Y (28-bit reference Y); on lines 0-191 the internal reference is reloaded too.
    pub fn set_bg2y(&mut self, word: u32, vcount: u16) {
        self.bg2y = word;
        if vcount < 192 {
            self.bg2y_internal = word as i32;
        }
    }

    /// Sets BG3X (28-bit reference X); on lines 0-191 the internal reference is reloaded too.
    pub fn set_bg3x(&mut self, word: u32, vcount: u16) {
        self.bg3x = word;
        if vcount < 192 {
            self.bg3x_internal = word as i32;
        }
    }

    /// Sets BG3Y (28-bit reference Y); on lines 0-191 the internal reference is reloaded too.
    pub fn set_bg3y(&mut self, word: u32, vcount: u16) {
        self.bg3y = word;
        if vcount < 192 {
            self.bg3y_internal = word as i32;
        }
    }

    /// Stores WIN0H (left/right edges).
    pub fn set_win0h(&mut self, halfword: u16) {
        self.win0h = halfword;
    }

    /// Stores WIN1H (left/right edges).
    pub fn set_win1h(&mut self, halfword: u16) {
        self.win1h = halfword;
    }

    /// Stores WIN0V (top/bottom edges).
    pub fn set_win0v(&mut self, halfword: u16) {
        self.win0v = halfword;
    }

    /// Stores WIN1V (top/bottom edges).
    pub fn set_win1v(&mut self, halfword: u16) {
        self.win1v = halfword;
    }

    /// Stores MOSAIC.
    pub fn set_mosaic(&mut self, halfword: u16) {
        self.mosaic = halfword;
    }

    /// Decodes WININ into per-window layer / effect flags.
    pub fn set_winin(&mut self, halfword: u16) {
        for bit in 0..4 {
            self.winin.win0_bg_enabled[bit] = (halfword & (1 << bit)) != 0;
            self.winin.win1_bg_enabled[bit] = (halfword & (1 << (bit + 8))) != 0;
        }
        // LCD I/O Window Feature: https://problemkaputt.de/gbatek.htm#lcdiowindowfeature
        self.winin.win0_obj_enabled = (halfword & (1 << 4)) != 0; // WININ bit 4: win0 obj enabled
        self.winin.win0_color_special = (halfword & (1 << 5)) != 0; // WININ bit 5: win0 color special
        self.winin.win1_obj_enabled = (halfword & (1 << 12)) != 0; // WININ bit 12: win1 obj enabled
        self.winin.win1_color_special = (halfword & (1 << 13)) != 0; // WININ bit 13: win1 color special
    }

    /// Decodes WINOUT into outside / OBJ-window layer flags.
    pub fn set_winout(&mut self, halfword: u16) {
        for bit in 0..4 {
            self.winout.outside_bg_enabled[bit] = (halfword & (1 << bit)) != 0;
            self.winout.objwin_bg_enabled[bit] = (halfword & (1 << (bit + 8))) != 0;
        }
        // LCD I/O Window Feature: https://problemkaputt.de/gbatek.htm#lcdiowindowfeature
        self.winout.outside_obj_enabled = (halfword & (1 << 4)) != 0; // WINOUT bit 4: outside obj enabled
        self.winout.outside_color_special = (halfword & (1 << 5)) != 0; // WINOUT bit 5: outside color special
        self.winout.objwin_obj_enabled = (halfword & (1 << 12)) != 0; // WINOUT bit 12: objwin obj enabled
        self.winout.objwin_color_special = (halfword & (1 << 13)) != 0; // WINOUT bit 13: objwin color special
    }

    /// Decodes BLDCNT (first/second targets, effect).
    pub fn set_bldcnt(&mut self, halfword: u16) {
        for bit in 0..4 {
            self.bldcnt.bg_first_target_pix[bit] = (halfword & (1 << bit)) != 0;
            self.bldcnt.bg_second_target_pix[bit] = (halfword & (1 << (bit + 8))) != 0;
        }
        // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
        self.bldcnt.obj_first_target_pix = (halfword & (1 << 4)) != 0; // BLDCNT bit 4: obj first target pix
        self.bldcnt.obj_second_target_pix = (halfword & (1 << 12)) != 0; // BLDCNT bit 12: obj second target pix

        self.bldcnt.bd_first_target_pix = (halfword & (1 << 5)) != 0; // BLDCNT bit 5: bd first target pix
        self.bldcnt.bd_second_target_pix = (halfword & (1 << 13)) != 0; // BLDCNT bit 13: bd second target pix

        // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
        self.bldcnt.effect = ((halfword >> 6) & 0x3) as u8; // color special effect (bits 6-7)
    }

    /// Stores BLDALPHA (EVA/EVB).
    pub fn set_bldalpha(&mut self, halfword: u16) {
        self.bldalpha = halfword;
    }

    /// Stores BLDY (EVY).
    pub fn set_bldy(&mut self, byte: u8) {
        self.bldy = byte;
    }

    /// Stores MASTER_BRIGHT (mode bits 14-15, factor 0-16).
    pub fn set_master_bright(&mut self, halfword: u16) {
        self.master_bright = halfword;
    }

    /// Decodes DISPCAPCNT (EVA/EVB, write bank/offset, size, sources, enable).
    pub fn set_dispcapcnt(&mut self, word: u32) {
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

        // DS Video Capture and Main Memory Display Mode: https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode
        self.dispcapcnt.b_display_fifo = (word & (1 << 25)) != 0; // DISPCAPCNT bit 25: b display fifo

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
