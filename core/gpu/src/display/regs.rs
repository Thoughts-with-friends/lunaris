// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
/// Scheduler event entry for timing-based events
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
pub struct SchedulerEvent {
    pub id: i32,
    pub processing: bool,
    pub activation_time: u64,
}

/// Display status register for screen state
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct DispStatReg {
    /// Currently in VBLANK period
    pub is_vblank: bool,
    /// Currently in HBLANK period
    pub is_hblank: bool,
    /// VCOUNTER matches VCOUNTER setting
    pub is_vcounter: bool,
    /// Interrupt on VBLANK enable
    pub irq_on_vblank: bool,
    /// Interrupt on HBLANK enable
    pub irq_on_hblank: bool,
    /// Interrupt on VCOUNTER enable
    pub irq_on_vcounter: bool,
    /// Current line counter comparison value
    pub vcounter: u16,
}

impl DispStatReg {
    /// Create new display status register
    pub fn new() -> Self {
        DispStatReg {
            is_vblank: false,
            is_hblank: false,
            is_vcounter: false,
            irq_on_vblank: false,
            irq_on_hblank: false,
            irq_on_vcounter: false,
            vcounter: 0,
        }
    }

    /// Get register value as 16-bit halfword
    pub fn get(&self) -> u16 {
        let mut value = 0u16;
        if self.is_vblank {
            value |= 1;
        }
        if self.is_hblank {
            value |= 2;
        }
        if self.is_vcounter {
            value |= 4;
        }
        if self.irq_on_vblank {
            value |= 8;
        }
        if self.irq_on_hblank {
            value |= 16;
        }
        if self.irq_on_vcounter {
            value |= 32;
        }
        // LCD I/O Interrupts and Status: https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus
        value |= (self.vcounter & 0xFF) << 8; // DISPSTAT V-count setting (bits 8-15)
        value
    }

    /// Set register value from 16-bit halfword
    pub fn set(&mut self, value: u16) {
        self.is_vblank = (value & 1) != 0;
        self.is_hblank = (value & 2) != 0;
        self.is_vcounter = (value & 4) != 0;
        self.irq_on_vblank = (value & 8) != 0;
        self.irq_on_hblank = (value & 16) != 0;
        self.irq_on_vcounter = (value & 32) != 0;
        self.vcounter = (value >> 8) & 0xFF; // DISPSTAT V-count setting (bits 8-15)
    }
}

impl Default for DispStatReg {
    fn default() -> Self {
        Self::new()
    }
}

/// Power control register for GPU features
///
/// POWCNT1 - Graphics Power Control Register (R/W)
///
/// - https://problemkaputt.de/gbatek.htm#dsiomaps
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct PowerCtrlReg {
    /// LCD display enable
    pub lcd_enable: bool,
    /// Engine A power (2D upper screen)
    pub engine_upper: bool,
    /// 3D rendering enable
    pub rendering_3d: bool,
    /// 3D geometry enable
    pub geometry_3d: bool,
    /// Engine B power (2D lower screen)
    pub engine_lower: bool,
    /// POWCNT1 bit 15: 1 = engine A drives the upper screen, 0 = lower.
    pub swap_display: bool,
}

impl PowerCtrlReg {
    /// Create new power control register
    pub fn new() -> Self {
        PowerCtrlReg {
            lcd_enable: true,
            engine_upper: true,
            rendering_3d: true,
            geometry_3d: true,
            engine_lower: true,
            swap_display: false,
        }
    }

    /// Get register value as 16-bit halfword
    pub fn get(&self) -> u16 {
        let mut value = 0u16;
        if self.lcd_enable {
            value |= 1;
        }
        if self.engine_upper {
            value |= 2;
        }
        if self.rendering_3d {
            value |= 4;
        }
        if self.geometry_3d {
            value |= 8;
        }
        if self.engine_lower {
            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            value |= 1 << 9; // POWCNT1 bit 9: 2D graphics engine B enable
        }
        if self.swap_display {
            value |= 1 << 15; // POWCNT1 bit 15: display swap (engine A on top)
        }
        value
    }

    /// Set register value from 16-bit halfword
    pub fn set(&mut self, value: u16) {
        self.lcd_enable = (value & 1) != 0;
        self.engine_upper = (value & 2) != 0;
        self.rendering_3d = (value & 4) != 0;
        self.geometry_3d = (value & 8) != 0;
        // GBATEK "DS Power Control": bit 9 = 2D engine B, bit 15 = display
        // swap (1 = engine A on the upper screen).
        self.engine_lower = (value & (1 << 9)) != 0;
        // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
        self.swap_display = (value & (1 << 15)) != 0; // POWCNT1 bit 15: display swap (engine A on top)
    }
}

impl Default for PowerCtrlReg {
    fn default() -> Self {
        Self::new()
    }
}
