//! 2D engine registers: DISPCNT, DISPCAPCNT, WININ / WINOUT, BLDCNT.
//! See GBATEK "DS Video Display / BG / Windows / Color Special Effects".
/// Display Control Register (DISPCNT)
///
/// Controls the main display settings, background visibility,
/// object rendering, and display modes.
#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone, Copy)]
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
#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone, Copy)]
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
#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone, Copy)]
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
#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone, Copy)]
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
#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone, Copy)]
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
