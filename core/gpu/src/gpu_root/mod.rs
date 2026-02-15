// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
// Graphics Processing Unit (GPU) implementation for Nintendo DS
/// Handles 2D and 3D rendering, VRAM management, and display output
pub(crate) mod arm_rw;
pub(crate) mod draw_scanline;
pub(crate) mod draw_scanline_3d;
pub(crate) mod draw_sprite;
pub(crate) mod getter;
pub(crate) mod matrix;
pub(crate) mod reader;
pub mod register;
pub(crate) mod render_2d;
pub(crate) mod setter;
pub(crate) mod vram_reader;
pub(crate) mod writer;

use crate::gpu_2d::Gpu2DEngine;
use crate::gpu_3d::structs::Gpu3D;
use crate::gpu_root::register::{DispStatReg, PowerCtrlReg, VramBankCfg};
use lunaris_ds_mem_const::{
    VRAM_A_SIZE, VRAM_B_SIZE, VRAM_C_SIZE, VRAM_D_SIZE, VRAM_E_SIZE, VRAM_F_SIZE, VRAM_G_SIZE,
    VRAM_H_SIZE, VRAM_I_SIZE,
};

/// Graphics Processing Unit
/// Manages 2D and 3D rendering for both screens
#[derive(Debug)]
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
    cycles: u64,

    /// VRAM Banks A-I (9 separate memory banks)
    vram_a: Vec<u8>,
    vram_b: Vec<u8>,
    vram_c: Vec<u8>,
    vram_d: Vec<u8>,
    vram_e: Vec<u8>,
    vram_f: Vec<u8>,
    vram_g: Vec<u8>,
    vram_h: Vec<u8>,
    vram_i: Vec<u8>,

    /// Palette memory for engine A (1024 bytes)
    palette_upper: Vec<u8>,
    /// Palette memory for engine B (1024 bytes)
    palette_lower: Vec<u8>,

    /// OAM (Object Attribute Memory) for sprites (2KB)
    oam: Vec<u8>,

    /// Display status register for ARM7
    pub display_status_arm7: DispStatReg,
    /// Display status register for ARM9
    pub display_status_arm9: DispStatReg,

    /// Current vertical line counter
    pub vertical_count: u16,

    /// VRAM bank configuration A-I
    vramcnt_a: VramBankCfg,
    vramcnt_b: VramBankCfg,
    vramcnt_c: VramBankCfg,
    vramcnt_d: VramBankCfg,
    vramcnt_e: VramBankCfg,
    vramcnt_f: VramBankCfg,
    vramcnt_g: VramBankCfg,
    vramcnt_h: VramBankCfg,
    vramcnt_i: VramBankCfg,

    /// Power control register
    power_control_reg: PowerCtrlReg,
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

            vram_a: vec![0u8; VRAM_A_SIZE as usize],
            vram_b: vec![0u8; VRAM_B_SIZE as usize],
            vram_c: vec![0u8; VRAM_C_SIZE as usize],
            vram_d: vec![0u8; VRAM_D_SIZE as usize],
            vram_e: vec![0u8; VRAM_E_SIZE as usize],
            vram_f: vec![0u8; VRAM_F_SIZE as usize],
            vram_g: vec![0u8; VRAM_G_SIZE as usize],
            vram_h: vec![0u8; VRAM_H_SIZE as usize],
            vram_i: vec![0u8; VRAM_I_SIZE as usize],

            palette_upper: vec![0u8; 1024],
            palette_lower: vec![0u8; 1024],

            oam: vec![0u8; 2048],

            display_status_arm7: DispStatReg::new(),
            display_status_arm9: DispStatReg::new(),

            vertical_count: 0,

            vramcnt_a: VramBankCfg::new(),
            vramcnt_b: VramBankCfg::new(),
            vramcnt_c: VramBankCfg::new(),
            vramcnt_d: VramBankCfg::new(),
            vramcnt_e: VramBankCfg::new(),
            vramcnt_f: VramBankCfg::new(),
            vramcnt_g: VramBankCfg::new(),
            vramcnt_h: VramBankCfg::new(),
            vramcnt_i: VramBankCfg::new(),

            power_control_reg: PowerCtrlReg::new(),
        }
    }
}

// TODO: Rename bytes_to_u16_slice()
pub fn bytes_to_palette(bytes: &[u8]) -> &[u16] {
    assert_eq!(bytes.len() % 2, 0, "u8 vector length must be even");
    unsafe { core::slice::from_raw_parts(bytes.as_ptr() as *const u16, bytes.len() / 2) }
}

/// # Panics
/// Panics if address is out of bounds(<= 0x3ff == 1023) for palette memory
pub fn read_palette_value(bytes: &[u8], address: u32) -> u16 {
    let off = (address & 0x3ff) as usize;
    u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap())
}

impl Gpu {
    pub fn draw_bg_txt_line(&self, index: i32, engine_a: bool) {
        let (_, _) = (index, engine_a);
        unimplemented!("C++ code is empty.")
    }
    pub fn draw_bg_extended_line(&self, index: i32, engine_a: bool) {
        let (_, _) = (index, engine_a);
        unimplemented!("C++ code is empty.")
    }
    pub fn draw_sprite_line(&self, engine_a: bool) {
        let _ = engine_a;
        unimplemented!("C++ code is empty.")
    }

    // moved draw_scanline.rs
    // pub fn draw_scanline(&self) {}
    // pub fn draw_3d_scanline(&mut self, is_engine_a: bool, bg_priority: u8)

    /// Get current cycle count
    pub fn get_cycles(&self) -> u64 {
        self.cycles
    }

    /// Power on GPU
    pub fn power_on(&mut self) {
        // self.eng_3d.power_on();
        self.cycles = 0;
        self.frame_complete = false;
        self.frames_skipped = 0;

        self.set_powcnt1(0x820F);
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

        self.vram_a.clear();
        self.vram_b.clear();
        self.vram_c.clear();
        self.vram_d.clear();
        self.vram_e.clear();
        self.vram_f.clear();
        self.vram_g.clear();
        self.vram_h.clear();
        self.vram_i.clear();
    }

    // moved struct Emulator;
    // pub fn handle_event(&self, event: &SchedulerEvent);

    /// Get upper screen framebuffer data
    #[inline]
    pub fn get_upper_frame(&self, buffer: &mut [u32]) {
        let engine = match self.power_control_reg.swap_display {
            true => &self.engine_upper,
            false => &self.engine_lower,
        };
        engine.get_framebuffer(buffer);
    }

    /// Get lower screen framebuffer data
    #[inline]
    pub fn get_lower_frame(&self, buffer: &mut [u32]) {
        let engine = match self.power_control_reg.swap_display {
            true => &self.engine_lower,
            false => &self.engine_upper,
        };
        engine.get_framebuffer(buffer);
    }

    /// Mark frame start
    pub fn start_frame(&mut self) {
        self.frame_complete = false;
    }

    /// Mark frame completion
    pub fn end_frame(&mut self) {
        self.frame_complete = true;
    }

    // moved gpu_3d.rs in lunaris_emu/gpu
    // fn check_gxfifo_dma(&self)
    // fn check_gxfifo_irq(&self)

    /// Check if current frame is complete
    pub fn is_frame_complete(&self) -> bool {
        self.frame_complete
    }

    /// Check if display screens are swapped
    pub fn display_swapped(&self) -> bool {
        self.power_control_reg.swap_display
    }

    /// Read from palette A
    pub fn read_palette_a(&self, address: u32) -> u16 {
        let address = address as usize;
        if address + 1 < self.palette_upper.len() {
            let lo = self.palette_upper[address] as u16;
            let hi = self.palette_upper[address + 1] as u16;
            lo | (hi << 8)
        } else {
            0
        }
    }

    // moved vram_reader.rs
    // pub fn read_extpal_bga(&self, address: u32) -> u16
    // pub fn read_extpal_obja(&self, address: u32) -> u16
    // pub fn read_extpal_objb(&self, address: u32) -> u16
    // pub fn read_extpal_bgb(&self, address: u32) -> u16

    /// Read from palette B
    pub fn read_palette_b(&self, address: u32) -> u16 {
        let address = address as usize;
        if address + 1 < self.palette_lower.len() {
            let lo = self.palette_lower[address] as u16;
            let hi = self.palette_lower[address + 1] as u16;
            lo | (hi << 8)
        } else {
            0
        }
    }

    // moved arm_rw.rs
    // pub fn read_arm7(&self, address: u32)
    // pub fn write_arm7<T>(&mut self, address: u32, value: T);

    // ===== Helpers =====

    pub fn read_vec_test(&self, address: u32) -> u16 {
        self.engine_3d.read_vec_test(address)
    }

    pub fn read_clip_mtx(&mut self, address: u32) -> u32 {
        self.engine_3d.read_clip_mtx(address)
    }

    pub fn read_vec_mtx(&self, address: u32) -> u32 {
        self.engine_3d.read_vec_mtx(address)
    }
}
