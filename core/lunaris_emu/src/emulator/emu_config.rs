// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! emulator.hpp
//!
use lunaris_ds_mem_const::*;

#[derive(Debug)]
pub struct Config {
    pub arm7_bios_path: String,
    pub arm9_bios_path: String,
    pub firmware_path: String,
    pub savelist_path: String,

    /// Enable direct boot
    pub direct_boot_enabled: bool,

    /// Pause emulator when window is unfocused
    pub pause_when_unfocused: bool,

    /// Background enable flags
    pub bg_enable: [bool; 4],
    pub frameskip: u32,

    /// Enable frame limiter
    pub enable_framelimiter: bool,

    /// Use HLE BIOS
    pub hle_bios: bool,

    /// Test mode
    pub test: bool,
}

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    pub fn new() -> Self {
        Self {
            arm7_bios_path: Default::default(),
            arm9_bios_path: Default::default(),
            firmware_path: Default::default(),
            savelist_path: Default::default(),
            direct_boot_enabled: true,
            pause_when_unfocused: Default::default(),
            bg_enable: Default::default(),
            frameskip: Default::default(),
            enable_framelimiter: Default::default(),
            hle_bios: Default::default(),
            test: Default::default(),
        }
    }
}

/// Button input register for standard DS buttons
#[derive(Debug, Clone, Copy, Default)]
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
    /// Get the current key input register value (bit-packed format)
    pub fn get_value(&self) -> u16 {
        let mut value = 0u16;
        if self.button_a {
            value |= 0x0001;
        }
        if self.button_b {
            value |= 0x0002;
        }
        if self.select {
            value |= 0x0004;
        }
        if self.start {
            value |= 0x0008;
        }
        if self.right {
            value |= 0x0010;
        }
        if self.left {
            value |= 0x0020;
        }
        if self.up {
            value |= 0x0040;
        }
        if self.down {
            value |= 0x0080;
        }
        if self.button_r {
            value |= 0x0100;
        }
        if self.button_l {
            value |= 0x0200;
        }
        value
    }

    /// Set value from bit-packed register format
    pub fn set_value(&mut self, value: u16) {
        self.button_a = (value & 0x0001) != 0;
        self.button_b = (value & 0x0002) != 0;
        self.select = (value & 0x0004) != 0;
        self.start = (value & 0x0008) != 0;
        self.right = (value & 0x0010) != 0;
        self.left = (value & 0x0020) != 0;
        self.up = (value & 0x0040) != 0;
        self.down = (value & 0x0080) != 0;
        self.button_r = (value & 0x0100) != 0;
        self.button_l = (value & 0x0200) != 0;
    }
}

/// Extended key input register for additional buttons (X, Y, pen, hinge)
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtKeyInReg {
    pub button_x: bool,
    pub button_y: bool,
    pub pen_down: bool,
    pub hinge_closed: bool,
}

impl ExtKeyInReg {
    /// Get the extended key input register value
    pub fn get_value(&self) -> u16 {
        let mut value = 0u16;
        if self.button_x {
            value |= 0x0001;
        }
        if self.button_y {
            value |= 0x0002;
        }
        if self.pen_down {
            value |= 0x0004;
        }
        if self.hinge_closed {
            value |= 0x0008;
        }
        value
    }

    /// Set value from bit-packed register format
    pub fn set_value(&mut self, value: u16) {
        self.button_x = (value & 0x0001) != 0;
        self.button_y = (value & 0x0002) != 0;
        self.pen_down = (value & 0x0004) != 0;
        self.hinge_closed = (value & 0x0008) != 0;
    }
}

/// Power control register for power management
#[derive(Debug, Clone, Copy, Default)]
pub struct PowCnt2Reg {
    pub speakers: bool,
    pub wifi: bool,
    pub led: bool,
    pub cartridge: bool,
}

impl PowCnt2Reg {
    /// Get the power control register value
    pub fn get_value(&self) -> u16 {
        let mut value = 0u16;
        if self.speakers {
            value |= 0x0001;
        }
        if self.wifi {
            value |= 0x0002;
        }
        if self.led {
            value |= 0x0004;
        }
        if self.cartridge {
            value |= 0x0008;
        }
        value
    }

    /// Set value from bit-packed register format
    pub fn set_value(&mut self, value: u16) {
        self.speakers = (value & 0x0001) != 0;
        self.wifi = (value & 0x0002) != 0;
        self.led = (value & 0x0004) != 0;
        self.cartridge = (value & 0x0008) != 0;
    }
}

#[derive(Debug)]
pub enum BiosMem<const BIOS_SIZE: usize> {
    /// len: BIOS_SIZE
    User(Vec<u8>),
    Const(&'static [u8; BIOS_SIZE]),
}

impl<const BIOS_SIZE: usize> BiosMem<BIOS_SIZE> {
    pub fn get(&self, range: core::ops::Range<usize>) -> Option<&[u8]> {
        match self {
            Self::User(items) => items.get(range),
            Self::Const(items) => items.get(range),
        }
    }
}

impl Default for BiosMem<BIOS7_SIZE> {
    fn default() -> Self {
        BiosMem::Const(&lunaris_ds_free_bios::arm7::BIOS_ARM7_BIN)
    }
}

impl Default for BiosMem<BIOS9_SIZE> {
    fn default() -> Self {
        BiosMem::Const(&lunaris_ds_free_bios::arm9::BIOS_ARM9_BIN)
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
