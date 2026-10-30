// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
// Graphics Processing Unit (GPU) implementation for Nintendo DS
/// Handles 2D and 3D rendering, VRAM management, and display output
use crate::engine_2d::Gpu2DEngine;
use crate::{
    display::{
        hires::HdLine,
        regs::{DispStatReg, PowerCtrlReg},
    },
    engine_3d::state::Gpu3D,
    vram::Vram,
};

/// Graphics Processing Unit
/// Manages 2D and 3D rendering for both screens
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Gpu {
    /// 2D Engine upper screen
    pub engine_upper: Gpu2DEngine,
    /// 2D Engine lower screen
    pub engine_lower: Gpu2DEngine,

    pub engine_3d: Gpu3D,

    /// Frame rendering complete flag
    pub frame_complete: bool,
    /// Number of frames skipped for frame skip
    pub frames_skipped: u32,

    /// Cycle counter
    pub(crate) cycles: u64,

    /// VRAM banks A-I and their mapping
    pub vram: Vram,

    /// Palette memory for engine A (1024 bytes)
    pub(crate) palette_upper: Vec<u8>,
    /// Palette memory for engine B (1024 bytes)
    pub(crate) palette_lower: Vec<u8>,

    /// OAM (Object Attribute Memory) for sprites (2KB)
    pub(crate) oam: Vec<u8>,

    /// Display status register for ARM7
    pub display_status_arm7: DispStatReg,
    /// Display status register for ARM9
    pub display_status_arm9: DispStatReg,

    /// Current vertical line counter
    pub vertical_count: u16,

    /// Power control register
    pub(crate) power_control_reg: PowerCtrlReg,

    /// Engine A, per pixel: how a visible 3D pixel was composited
    /// (`display::hires`).
    pub(crate) hd_3d: Vec<u32>,
    /// Engine A, per line: display mode, brightness and BG0 scroll.
    #[serde(with = "serde_big_array::BigArray")]
    pub(crate) hd_lines: [HdLine; ds_mem_addr::SCANLINES],
}

impl Default for Gpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Gpu {
    /// Create new GPU instance
    pub fn new() -> Self {
        Gpu {
            engine_upper: Gpu2DEngine::new(),
            engine_lower: Gpu2DEngine::new(),
            engine_3d: Gpu3D::new(),

            frame_complete: false,
            frames_skipped: 0,

            cycles: 0,

            vram: Vram::new(),

            palette_upper: vec![0u8; 1024],
            palette_lower: vec![0u8; 1024],

            oam: vec![0u8; 2048],

            display_status_arm7: DispStatReg::new(),
            display_status_arm9: DispStatReg::new(),

            vertical_count: 0,

            power_control_reg: PowerCtrlReg::new(),

            hd_3d: vec![0; ds_mem_addr::PIXELS_PER_LINE * ds_mem_addr::SCANLINES],
            hd_lines: [HdLine::default(); ds_mem_addr::SCANLINES],
        }
    }
}

// TODO: Rename bytes_to_u16_slice()

/// # Panics
/// Panics if address is out of bounds(<= 0x3ff == 1023) for palette memory
pub fn read_palette_value(bytes: &[u8], address: u32) -> u16 {
    // LCD Color Palettes: https://problemkaputt.de/gbatek.htm#lcdcolorpalettes
    let off = (address & 0x3ff) as usize; // 1K palette RAM per engine
    u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap())
}

impl Gpu {
    // moved draw_scanline.rs
    // pub fn draw_scanline(&self) {}
    // pub fn draw_3d_scanline(&mut self, is_engine_a: bool, bg_priority: u8)

    /// Resets registers, VRAM mapping, palettes and OAM (from ds_emu
    /// power_on()). POWCNT1 = 0x820F: both engines and 3D on, engine A on
    /// the top screen.
    pub fn power_on(&mut self) {
        // self.eng_3d.power_on();
        self.cycles = 0;
        self.frame_complete = false;
        self.frames_skipped = 0;

        // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
        self.set_powcnt1(0x820F); // POWCNT1: LCDs, 2D A, 3D render, 3D geometry, 2D B, display swap
        self.set_dispcnt_a(0);
        self.set_dispcnt_b(0);
        self.set_dispstat7(0);
        self.set_dispstat9(0);
        self.set_win0h_a(0);
        self.set_dispcapcnt(0);

        for i in 0..4 {
            self.set_bghofs_a(0, i);
            self.set_bgvofs_a(0, i);
            self.set_bghofs_b(0, i);
            self.set_bgvofs_b(0, i);
        }

        self.vram.reset();
        self.palette_upper.fill(0);
        self.palette_lower.fill(0);
        self.oam.fill(0);
    }

    // moved struct Emulator;
    // pub fn handle_event(&self, event: &SchedulerEvent);

    /// Get upper screen framebuffer data.
    ///
    /// POWCNT1 bit 15 (`swap_display`) selects which engine drives which
    /// screen: set = engine A on the upper screen (the usual configuration,
    /// 0x820F after boot), clear = engine A on the lower screen.
    #[inline]
    pub fn get_upper_frame(&self, buffer: &mut [u32]) {
        let engine = match self.power_control_reg.swap_display {
            true => &self.engine_upper,
            false => &self.engine_lower,
        };
        engine.get_framebuffer(buffer);
    }

    /// Get lower screen framebuffer data. See `get_upper_frame`.
    #[inline]
    pub fn get_lower_frame(&self, buffer: &mut [u32]) {
        let engine = match self.power_control_reg.swap_display {
            true => &self.engine_lower,
            false => &self.engine_upper,
        };
        engine.get_framebuffer(buffer);
    }

    /// Clears `frame_complete` (ds_emu run() step 1).
    pub fn start_frame(&mut self) {
        self.frame_complete = false;
    }

    /// Sets `frame_complete` (the scheduler sets the field directly at
    /// VBlank).
    pub fn end_frame(&mut self) {
        self.frame_complete = true;
    }

    // moved gpu_3d.rs in lunaris_emu/gpu
    // fn check_gxfifo_dma(&self)
    // fn check_gxfifo_irq(&self)

    /// Whether VBlank started; ends the ds_emu run() loop.
    pub fn is_frame_complete(&self) -> bool {
        self.frame_complete
    }

    /// Check if display screens are swapped
    pub fn display_swapped(&self) -> bool {
        self.power_control_reg.swap_display
    }

    /// Read from palette A (`address` may be absolute; only bits 0-9 count).
    pub fn read_palette_a(&self, address: u32) -> u16 {
        read_palette_value(&self.palette_upper, address & 0x3FE) // halfword-aligned offset in 1K palette RAM
    }

    // moved vram_reader.rs
    // pub fn read_extpal_bga(&self, address: u32) -> u16
    // pub fn read_extpal_obja(&self, address: u32) -> u16
    // pub fn read_extpal_objb(&self, address: u32) -> u16
    // pub fn read_extpal_bgb(&self, address: u32) -> u16

    /// Read from palette B (`address` may be absolute; only bits 0-9 count).
    pub fn read_palette_b(&self, address: u32) -> u16 {
        read_palette_value(&self.palette_lower, address & 0x3FE) // halfword-aligned offset in 1K palette RAM
    }

    // moved arm_rw.rs
    // pub fn read_arm7(&self, address: u32)
    // pub fn write_arm7<T>(&mut self, address: u32, value: T);
}
