// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
/// Scheduler event entry for timing-based events
#[derive(Debug, Clone, Default)]
pub struct SchedulerEvent {
    pub id: i32,
    pub processing: bool,
    pub activation_time: u64,
}

/// Display status register for screen state
#[derive(Debug, Clone, Copy)]
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
        value |= (self.vcounter & 0xFF) << 8;
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
        self.vcounter = (value >> 8) & 0xFF;
    }
}

impl Default for DispStatReg {
    fn default() -> Self {
        Self::new()
    }
}

/// VRAM bank configuration
#[derive(Debug, Clone, Copy)]
pub struct VramBankCfg {
    /// Master select mode for this bank
    pub mst: u32,
    /// Offset within mode
    pub offset: u32,
    /// Is this bank enabled
    pub enabled: bool,
}

impl VramBankCfg {
    /// Create new VRAM bank configuration
    pub fn new() -> Self {
        VramBankCfg {
            mst: 0,
            offset: 0,
            enabled: false,
        }
    }
}

impl Default for VramBankCfg {
    fn default() -> Self {
        Self::new()
    }
}

/// Power control register for GPU features
///
/// POWCNT1 - Graphics Power Control Register (R/W)
///
/// - https://problemkaputt.de/gbatek.htm#dsiomaps
#[derive(Debug, Clone, Copy)]
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
    /// Swap upper/lower display screens
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
            value |= 16;
        }
        if self.swap_display {
            value |= 32;
        }
        value
    }

    /// Set register value from 16-bit halfword
    pub fn set(&mut self, value: u16) {
        self.lcd_enable = (value & 1) != 0;
        self.engine_upper = (value & 2) != 0;
        self.rendering_3d = (value & 4) != 0;
        self.geometry_3d = (value & 8) != 0;
        self.engine_lower = (value & 16) != 0;
        self.swap_display = (value & 32) != 0;
    }
}

impl Default for PowerCtrlReg {
    fn default() -> Self {
        Self::new()
    }
}
