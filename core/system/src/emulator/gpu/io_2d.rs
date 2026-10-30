//! ARM9 access to the 2D engine registers (0x04000000 / 0x04001000 + 0x00..0x6F).
//!
//! Games write these registers with byte, halfword and word stores, and
//! several are wider than the access (DISPCNT, BGxX/BGxY, DISPCAPCNT are 32
//! bits). Every access is merged into a raw shadow copy first and the whole
//! register is then handed to the GPU, so partial writes never clobber the
//! other half (e.g. `STRH` to DISPCNT must keep the display mode in bits
//! 16-17; clearing it selects "display off", a white screen).
//!
//! Register layout follows GBATEK "DS Video" / melonDS `GPU2D::Write*`.
use crate::emulator::Emulator;

/// Size of one engine's register block in bytes.
pub const IO_2D_SIZE: usize = 0x70;

/// Whether `address` is a 2D engine register handled here.
///
/// DISPSTAT/VCOUNT (0x04000004..7) are shared registers and are excluded.
pub const fn is_2d_reg(address: u32) -> bool {
    let offset = address & 0xFFF;
    // DS Video BG Modes / Control: https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol
    (address & !0xFFF == 0x0400_0000 || address & !0xFFF == 0x0400_1000) // DISPCNT - LCD Control / Engine B DISPCNT - LCD Control
        && offset < IO_2D_SIZE as u32
        && !(offset >= 4 && offset < 8)
}

/// Engine index (0 = A, 1 = B) and register offset of a 2D register address.
const fn split(address: u32) -> (usize, usize) {
    // DS I/O Maps: https://problemkaputt.de/gbatek.htm#dsiomaps
    (((address >> 12) & 1) as usize, (address & 0xFFF) as usize) // bit 12 selects engine B (04001000h), low 12 bits = register offset
}

impl Emulator {
    /// Halfword of the raw register shadow of `engine`.
    fn io_2d_u16(&self, engine: usize, offset: usize) -> u16 {
        let regs = &self.io_2d[engine];
        u16::from_le_bytes([regs[offset & !1], regs[(offset & !1) + 1]])
    }

    /// Word of the raw register shadow of `engine`.
    fn io_2d_u32(&self, engine: usize, offset: usize) -> u32 {
        let lo = u32::from(self.io_2d_u16(engine, offset & !3));
        let hi = u32::from(self.io_2d_u16(engine, (offset & !3) + 2));
        lo | (hi << 16) // two 16-bit registers per word
    }

    /// Byte write to a 2D engine register.
    pub fn write_io_2d_u8(&mut self, address: u32, value: u8) {
        let (engine, offset) = split(address);
        self.sync_dispcapcnt(engine, offset);
        self.io_2d[engine][offset] = value;
        self.apply_io_2d(engine, offset & !1);
    }

    /// Halfword write to a 2D engine register.
    ///
    /// ```text
    /// from: arm9_write_halfword() when is_2d_reg(address)
    ///   |   (u8 / u32 variants: arm9_write_byte / arm9_write_word)
    ///   v
    /// write_io_2d_u16(address, value)
    ///   1. split -> engine A (0x04000000) or B (0x04001000), offset
    ///   2. sync_dispcapcnt()       refresh the capture-busy bit first
    ///   3. merge `value` into the raw shadow io_2d[engine]
    ///   4. apply_io_2d()           decode the whole register into the
    ///                              GPU (gpu.set_dispcnt_a, set_bgcnt_*,
    ///                              scroll, rotation, windows, blending...)
    ///   |
    ///   v
    /// to:   the 2D engine uses the new state from the next draw_scanline()
    /// ```
    pub fn write_io_2d_u16(&mut self, address: u32, value: u16) {
        let (engine, offset) = split(address & !1);
        self.sync_dispcapcnt(engine, offset);
        self.io_2d[engine][offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        self.apply_io_2d(engine, offset);
    }

    /// Word write to a 2D engine register.
    pub fn write_io_2d_u32(&mut self, address: u32, value: u32) {
        let (engine, offset) = split(address & !3);
        self.sync_dispcapcnt(engine, offset);
        self.io_2d[engine][offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        // 32-bit registers are applied once with the whole value.
        self.apply_io_2d(engine, offset);
        // GBA I/O Map: https://problemkaputt.de/gbatek.htm#gbaiomap
        if !matches!(offset, 0x00 | 0x28 | 0x2C | 0x38 | 0x3C | 0x64) {
            // 32-bit registers: DISPCNT, BG2X, BG2Y, BG3X, BG3Y, DISPCAPCNT
            self.apply_io_2d(engine, offset + 2);
        }
    }

    /// DISPCAPCNT bit 31 is cleared by the hardware when a capture ends, so
    /// the shadow is refreshed from the GPU before a partial write merges
    /// into it.
    fn sync_dispcapcnt(&mut self, engine: usize, offset: usize) {
        // DS Video Capture and Main Memory Display Mode: https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode
        if engine == 0 && (0x64..0x68).contains(&offset) {
            // DISPCAPCNT (engine A only)
            let current = self.gpu.get_dispcapcnt_a();
            self.io_2d[0][0x64..0x68].copy_from_slice(&current.to_le_bytes()); // DISPCAPCNT
        }
    }

    /// Halfword read. Write-only registers read as 0 (GBATEK).
    /// Called by arm9_read_* for 2D registers; DISPCAPCNT and DISP3DCNT come
    /// from the live GPU state, the rest from the shadow.
    pub fn read_io_2d_u16(&self, address: u32) -> u16 {
        let (engine, offset) = split(address & !1);
        match offset {
            // DISPCNT, BGxCNT, WININ/WINOUT, BLDCNT/BLDALPHA, DISPCAPCNT,
            // MASTER_BRIGHT are readable.
            0x00..=0x03 | 0x08..=0x0F | 0x48..=0x4B | 0x50..=0x53 | 0x6C..=0x6D => {
                self.io_2d_u16(engine, offset)
            }
            0x64..=0x67 if engine == 0 => {
                // DISPCAPCNT (engine A only)
                let word = self.gpu.get_dispcapcnt_a();
                if offset & 2 == 0 { word as u16 } else { (word >> 16) as u16 }
            }
            // DISP3DCNT carries hardware status bits.
            0x60 if engine == 0 => self.gpu.get_disp3dcnt(),
            _ => 0,
        }
    }

    /// Byte read.
    pub fn read_io_2d_u8(&self, address: u32) -> u8 {
        let half = self.read_io_2d_u16(address & !1);
        if address & 1 == 0 { half as u8 } else { (half >> 8) as u8 }
    }

    /// Word read.
    pub fn read_io_2d_u32(&self, address: u32) -> u32 {
        let address = address & !3;
        u32::from(self.read_io_2d_u16(address))
            | (u32::from(self.read_io_2d_u16(address + 2)) << 16) // upper halfword
    }

    /// Forwards the (complete) register containing halfword `offset` to the GPU
    /// (last step of write_io_2d_u8/u16/u32).
    fn apply_io_2d(&mut self, engine: usize, offset: usize) {
        let half = self.io_2d_u16(engine, offset);
        // The full 32-bit register for DISPCNT, BGxX/BGxY and DISPCAPCNT.
        let word = self.io_2d_u32(engine, offset);
        let gpu = &mut self.gpu;
        let a = engine == 0;
        match offset {
            // LCD I/O Display Control: https://problemkaputt.de/gbatek.htm#lcdiodisplaycontrol
            0x00 | 0x02 => {
                // DISPCNT
                if a {
                    gpu.set_dispcnt_a(word);
                } else {
                    gpu.set_dispcnt_b(word);
                }
            }
            // LCD I/O BG Control: https://problemkaputt.de/gbatek.htm#lcdiobgcontrol
            0x08..=0x0E => {
                // BG0CNT-BG3CNT
                let index = (offset - 0x08) / 2; // BGxCNT index (2 bytes each)
                if a {
                    gpu.set_bgcnt_a(half, index);
                } else {
                    gpu.set_bgcnt_b(half, index);
                }
            }
            // LCD I/O BG Scrolling: https://problemkaputt.de/gbatek.htm#lcdiobgscrolling
            0x10..=0x1E => {
                // BG0HOFS-BG3VOFS
                let index = (offset - 0x10) / 4; // BGxHOFS/VOFS pair index (4 bytes per BG)
                match (a, offset & 2 == 0) {
                    (true, true) => gpu.set_bghofs_a(half, index),
                    (true, false) => gpu.set_bgvofs_a(half, index),
                    (false, true) => gpu.set_bghofs_b(half, index),
                    (false, false) => gpu.set_bgvofs_b(half, index),
                }
            }
            // LCD I/O BG Rotation/Scaling: https://problemkaputt.de/gbatek.htm#lcdiobgrotationscaling
            0x20..=0x26 => {
                // BG2PA-BG2PD
                let index = (offset - 0x20) / 2; // parameter index (2 bytes each)
                if a {
                    gpu.set_bg2p_a(half, index);
                } else {
                    gpu.set_bg2p_b(half, index);
                }
            }
            0x30..=0x36 => {
                // BG3PA-BG3PD
                let index = (offset - 0x30) / 2; // parameter index (2 bytes each)
                if a {
                    gpu.set_bg3p_a(half, index);
                } else {
                    gpu.set_bg3p_b(half, index);
                }
            }
            // BG2X/BG2Y/BG3X/BG3Y: 28-bit reference points; any write reloads
            // the internal counter.
            0x28..=0x2E | 0x38..=0x3E => match (a, offset & !3) {
                (true, 0x28) => gpu.set_bg2x_a(word), // BG2X
                (true, 0x2C) => gpu.set_bg2y_a(word), // BG2Y
                (true, 0x38) => gpu.set_bg3x_a(word), // BG3X
                (true, _) => gpu.set_bg3y_a(word),
                (false, 0x28) => gpu.set_bg2x_b(word), // BG2X
                (false, 0x2C) => gpu.set_bg2y_b(word), // BG2Y
                (false, 0x38) => gpu.set_bg3x_b(word), // BG3X
                (false, _) => gpu.set_bg3y_b(word),
            },
            // LCD I/O Window Feature: https://problemkaputt.de/gbatek.htm#lcdiowindowfeature
            0x40 => {
                // WIN0H
                if a {
                    gpu.set_win0h_a(half);
                } else {
                    gpu.set_win0h_b(half);
                }
            }
            0x42 => {
                // WIN1H
                if a {
                    gpu.set_win1h_a(half);
                } else {
                    gpu.set_win1h_b(half);
                }
            }
            0x44 => {
                // WIN0V
                if a {
                    gpu.set_win0v_a(half);
                } else {
                    gpu.set_win0v_b(half);
                }
            }
            0x46 => {
                // WIN1V
                if a {
                    gpu.set_win1v_a(half);
                } else {
                    gpu.set_win1v_b(half);
                }
            }
            0x48 => {
                // WININ
                if a {
                    gpu.set_winin_a(half);
                } else {
                    gpu.set_winin_b(half);
                }
            }
            0x4A => {
                // WINOUT
                if a {
                    gpu.set_winout_a(half);
                } else {
                    gpu.set_winout_b(half);
                }
            }
            // LCD I/O Mosaic Function: https://problemkaputt.de/gbatek.htm#lcdiomosaicfunction
            0x4C => {
                // MOSAIC
                if a {
                    gpu.set_mosaic_a(half);
                } else {
                    gpu.set_mosaic_b(half);
                }
            }
            // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
            0x50 => {
                // BLDCNT
                if a {
                    gpu.set_bldcnt_a(half);
                } else {
                    gpu.set_bldcnt_b(half);
                }
            }
            0x52 => {
                // BLDALPHA
                if a {
                    gpu.set_bldalpha_a(half);
                } else {
                    gpu.set_bldalpha_b(half);
                }
            }
            0x54 => {
                // BLDY
                if a {
                    gpu.set_bldy_a(half as u8);
                } else {
                    gpu.set_bldy_b(half as u8);
                }
            }
            // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
            0x60 if a => gpu.set_disp3dcnt(half), // DISP3DCNT (engine A only)
            // DS Video Capture and Main Memory Display Mode: https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode
            0x64 | 0x66 if a => {
                // DISPCAPCNT (engine A only)
                gpu.set_dispcapcnt(word);
            }
            // DS Video Stuff: https://problemkaputt.de/gbatek.htm#dsvideostuff
            0x6C => {
                // MASTER_BRIGHT
                if a {
                    gpu.set_master_bright_a(half);
                } else {
                    gpu.set_master_bright_b(half);
                }
            }
            // Unused / not emulated (0x4E, 0x56..0x5E, DISP_MMEM_FIFO 0x68).
            _ => {}
        }
    }
}
