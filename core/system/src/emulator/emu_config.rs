// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! emulator.hpp
//!
use ds_mem_addr::*;

#[derive(Debug)]
pub struct Config {
    pub arm7_bios_path: String,
    pub arm9_bios_path: String,
    pub firmware_path: String,
    pub savelist_path: String,
    /// Directory for `.sav` files. `None` keeps each save next to its ROM
    /// (`<rom>.sav`).
    pub save_dir: Option<std::path::PathBuf>,

    /// Enable direct boot
    pub direct_boot_enabled: bool,

    /// Raw FAT disk image used as the SD card of homebrew (DLDI).
    pub sd_image: Option<std::path::PathBuf>,
    /// Open the SD image read-only.
    pub sd_read_only: bool,

    /// Pause emulator when window is unfocused
    pub pause_when_unfocused: bool,

    /// Background enable flags
    pub bg_enable: [bool; 4],
    pub frameskip: u32,

    /// Enable frame limiter
    pub enable_framelimiter: bool,

    /// Use HLE BIOS.
    ///
    /// When `true`, `SWI` instructions are intercepted directly by
    /// [`crate::emulator::Emulator::hle_bios`] instead of jumping into the
    /// loaded/synthetic BIOS binary. The bundled `ds_bios`
    /// image is only a minimal placeholder (e.g. unimplemented routines
    /// like `IntrWait`/`VBlankIntrWait` are stub infinite loops), so real
    /// commercial ROMs — which rely on `SWI 5` every frame for vsync —
    /// hang permanently under LLE. HLE is therefore the default.
    pub hle_bios: bool,

    /// Test mode
    pub test: bool,

    /// Wi-Fi MAC address patched into the firmware at power-on. Two
    /// consoles playing together must not share one (the built-in
    /// firmware's is fixed). `None` keeps the firmware's.
    pub mac_address: Option<[u8; 6]>,

    /// Firmware user settings (nickname, birthday, language...) written
    /// into the firmware at power-on. `None` keeps the firmware's.
    pub user_settings: Option<ds_peripherals::spi::firmware::UserSettings>,

    /// Debugging aid: print every executed instruction of the selected CPU
    /// (bit 0 = ARM9, bit 1 = ARM7) to stderr.
    pub trace_cpus: u8,

    /// Debugging aid: `(cpu mask, pc)` — once that CPU is about to execute
    /// the instruction at `pc`, `trace_cpus` is set to the mask.
    pub trace_trigger: Option<(u8, u32)>,

    /// Debugging aid: print every CPU write to this word (main RAM mirrors
    /// are matched) to stderr.
    pub watch_addr: Option<u32>,
}

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    /// Default settings: no BIOS/firmware paths (built-in replacements),
    /// direct boot and HLE BIOS on, saves next to the ROM.
    pub fn new() -> Self {
        Self {
            arm7_bios_path: Default::default(),
            arm9_bios_path: Default::default(),
            firmware_path: Default::default(),
            savelist_path: Default::default(),
            save_dir: None,
            direct_boot_enabled: true,
            sd_image: None,
            sd_read_only: false,
            pause_when_unfocused: Default::default(),
            bg_enable: Default::default(),
            frameskip: Default::default(),
            enable_framelimiter: Default::default(),
            hle_bios: true,
            test: Default::default(),
            mac_address: None,
            user_settings: None,
            trace_cpus: 0,
            trace_trigger: None,
            watch_addr: None,
        }
    }
}

/// Button input register for standard DS buttons
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default)]
pub struct KeyInputReg {
    pub button_a: bool,
    pub button_b: bool,
    pub select: bool,
    pub start: bool,
    pub right: bool,
    pub left: bool,
    pub up: bool,
    pub down: bool,
    pub button_r: bool,
    pub button_l: bool,
}

impl KeyInputReg {
    /// Get the current key input register value (bit-packed format).
    ///
    /// KEYINPUT (4000130h) is active-low per GBATEK "DS Keypad": a cleared
    /// bit means the button is pressed, a set bit means released. All bits
    /// start at 1 (nothing pressed) and are cleared for each held button,
    /// matching CorgiDS `KEYINPUT_REG::get()`.
    pub fn get_value(&self) -> u16 {
        // GBA Keypad Input: https://problemkaputt.de/gbatek.htm#gbakeypadinput
        let mut value = 0x03FF_u16; // KEYINPUT: all 10 keys released (bits are 0 = pressed)
        if self.button_a {
            value &= !0x0001; // bit 0: A pressed
        }
        if self.button_b {
            value &= !0x0002; // bit 1: B pressed
        }
        if self.select {
            value &= !0x0004; // bit 2: Select pressed
        }
        if self.start {
            value &= !0x0008; // bit 3: Start pressed
        }
        if self.right {
            value &= !0x0010; // bit 4: Right pressed
        }
        if self.left {
            value &= !0x0020; // bit 5: Left pressed
        }
        if self.up {
            value &= !0x0040; // bit 6: Up pressed
        }
        if self.down {
            value &= !0x0080; // bit 7: Down pressed
        }
        if self.button_r {
            value &= !0x0100; // bit 8: R pressed
        }
        if self.button_l {
            value &= !0x0200; // bit 9: L pressed
        }
        value
    }

    /// Set value from bit-packed register format (active-low, see `get_value`)
    pub fn set_value(&mut self, value: u16) {
        self.button_a = (value & 0x0001) == 0; // bit 0: A (0 = pressed)
        self.button_b = (value & 0x0002) == 0; // bit 1: B (0 = pressed)
        self.select = (value & 0x0004) == 0; // bit 2: Select (0 = pressed)
        self.start = (value & 0x0008) == 0; // bit 3: Start (0 = pressed)
        self.right = (value & 0x0010) == 0; // bit 4: Right (0 = pressed)
        self.left = (value & 0x0020) == 0; // bit 5: Left (0 = pressed)
        self.up = (value & 0x0040) == 0; // bit 6: Up (0 = pressed)
        self.down = (value & 0x0080) == 0; // bit 7: Down (0 = pressed)
        self.button_r = (value & 0x0100) == 0; // bit 8: R (0 = pressed)
        self.button_l = (value & 0x0200) == 0; // bit 9: L (0 = pressed)
    }
}

/// Extended key input register for additional buttons (X, Y, pen, hinge)
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default)]
pub struct ExtKeyInReg {
    pub button_x: bool,
    pub button_y: bool,
    pub pen_down: bool,
    pub hinge_closed: bool,
}

impl ExtKeyInReg {
    /// Get the extended key input register value.
    ///
    /// EXTKEYIN (4000136h, ARM7-only) per GBATEK "DS Keypad": bit0=X,
    /// bit1=Y, bit6=pen state (active-low: 0 = touching), bit7=hinge state
    /// (active-high: 1 = closed). Unused bits read as 1. Matches CorgiDS
    /// `EXTKEYIN_REG::get()`.
    pub fn get_value(&self) -> u16 {
        // DS Keypad: https://problemkaputt.de/gbatek.htm#dskeypad
        let mut value = 0x007F_u16; // EXTKEYIN: X, Y, debug button, bit 6 pen up; bits 0-6 set = released
        if self.button_x {
            value &= !0x0001; // bit 0: X pressed
        }
        if self.button_y {
            value &= !0x0002; // bit 1: Y pressed
        }
        if self.pen_down {
            value &= !0x0040; // bit 6: pen down
        }
        if self.hinge_closed {
            value |= 0x0080; // bit 7: hinge closed
        }
        value
    }

    /// Set value from bit-packed register format (see `get_value` for polarity)
    pub fn set_value(&mut self, value: u16) {
        self.button_x = (value & 0x0001) == 0; // bit 0: X (0 = pressed)
        self.button_y = (value & 0x0002) == 0; // bit 1: Y (0 = pressed)
        self.pen_down = (value & 0x0040) == 0; // bit 6: pen down (0 = touching)
        self.hinge_closed = (value & 0x0080) != 0; // bit 7: hinge closed
    }
}

/// Power control register for power management
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default)]
pub struct PowCnt2Reg {
    pub speakers: bool,
    pub wifi: bool,
    pub led: bool,
    pub cartridge: bool,
}

impl PowCnt2Reg {
    /// Get the power control register value
    pub fn get_value(&self) -> u16 {
        let mut value = 0_u16;
        if self.speakers {
            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            value |= 0x0001; // POWCNT2 bit 0: sound speakers enable
        }
        if self.wifi {
            value |= 0x0002; // POWCNT2 bit 1: Wifi enable
        }
        if self.led {
            value |= 0x0004; // bit 2: not defined for POWCNT2 in GBATEK (kept by the emulator)
        }
        if self.cartridge {
            value |= 0x0008; // bit 3: not defined for POWCNT2 in GBATEK (kept by the emulator)
        }
        value
    }

    /// Set value from bit-packed register format
    pub fn set_value(&mut self, value: u16) {
        self.speakers = (value & 0x0001) != 0; // POWCNT2 bit 0: sound speakers enable
        self.wifi = (value & 0x0002) != 0; // POWCNT2 bit 1: Wifi enable
        self.led = (value & 0x0004) != 0; // bit 2: not defined for POWCNT2 in GBATEK
        self.cartridge = (value & 0x0008) != 0; // bit 3: not defined for POWCNT2 in GBATEK
    }
}

#[derive(Debug)]
pub enum BiosMem<const BIOS_SIZE: usize> {
    /// len: BIOS_SIZE
    User(Vec<u8>),
    Const(&'static [u8; BIOS_SIZE]),
}

impl<const BIOS_SIZE: usize> BiosMem<BIOS_SIZE> {
    /// Bytes `range` of the BIOS image (`None` when out of bounds or no
    /// dump is loaded).
    pub fn get(&self, range: core::ops::Range<usize>) -> Option<&[u8]> {
        match self {
            Self::User(items) => items.get(range),
            Self::Const(items) => items.get(range),
        }
    }
}

impl Default for BiosMem<BIOS7_SIZE> {
    fn default() -> Self {
        BiosMem::Const(&ds_bios::image::arm7::BIOS_ARM7_BIN)
    }
}

impl Default for BiosMem<BIOS9_SIZE> {
    fn default() -> Self {
        BiosMem::Const(&ds_bios::image::arm9::BIOS_ARM9_BIN)
    }
}

impl core::ops::Index<usize> for BiosMem<BIOS7_SIZE> {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        match self {
            BiosMem::User(items) => &items[index],
            BiosMem::Const(items) => &items[index],
        }
    }
}

impl core::ops::Index<core::ops::Range<usize>> for BiosMem<BIOS7_SIZE> {
    type Output = [u8];

    fn index(&self, index: core::ops::Range<usize>) -> &Self::Output {
        match self {
            BiosMem::User(items) => &items[index],
            BiosMem::Const(items) => &items[index],
        }
    }
}

impl core::ops::Index<usize> for BiosMem<BIOS9_SIZE> {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        match self {
            BiosMem::User(items) => &items[index],
            BiosMem::Const(items) => &items[index],
        }
    }
}

impl core::ops::Index<core::ops::Range<usize>> for BiosMem<BIOS9_SIZE> {
    type Output = [u8];

    fn index(&self, index: core::ops::Range<usize>) -> &Self::Output {
        match self {
            BiosMem::User(items) => &items[index],
            BiosMem::Const(items) => &items[index],
        }
    }
}
