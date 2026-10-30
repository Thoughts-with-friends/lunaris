// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
//! Engine-side and CPU-side VRAM accessors. All of them go through the
//! page-mapped [`crate::vram::Vram`]. BG/OBJ/LCDC/ARM7 accessors take
//! absolute addresses (e.g. `0x0620_0000 + x`; the region wraps them), the
//! 3D and extended palette accessors take offsets inside their slot space.
use crate::{
    Gpu,
    vram::{Region, Vram},
};

/// Generates `u8`/`u16`/`u32`/`u64` readers for one region.
macro_rules! region_readers {
    ($region:expr, $u8:ident, $u16:ident, $u32:ident, $u64:ident) => {
        #[inline]
        pub fn $u8(&self, address: u32) -> u8 {
            self.vram.read_u8($region, address)
        }
        #[inline]
        pub fn $u16(&self, address: u32) -> u16 {
            self.vram.read_u16($region, address)
        }
        #[inline]
        pub fn $u32(&self, address: u32) -> u32 {
            self.vram.read_u32($region, address)
        }
        #[inline]
        pub fn $u64(&self, address: u32) -> u64 {
            self.vram.read_u64($region, address)
        }
    };
}

impl Gpu {
    region_readers!(Region::BgA, read_bga_u8, read_bga_u16, read_bga_u32, read_bga_u64);
    region_readers!(Region::BgB, read_bgb_u8, read_bgb_u16, read_bgb_u32, read_bgb_u64);
    region_readers!(Region::ObjA, read_obja_u8, read_obja_u16, read_obja_u32, read_obja_u64);
    region_readers!(Region::ObjB, read_objb_u8, read_objb_u16, read_objb_u32, read_objb_u64);

    /// ARM9 VRAM read of any width at an absolute 0x06xx_xxxx address.
    #[inline]
    pub fn read_vram_arm9<const N: usize>(&self, address: u32) -> [u8; N] {
        let (region, offset) = Vram::arm9_region(address);
        self.vram.read(region, offset)
    }

    /// ARM9 VRAM write of any width at an absolute 0x06xx_xxxx address.
    #[inline]
    pub fn write_vram_arm9(&mut self, address: u32, bytes: &[u8]) {
        let (region, offset) = Vram::arm9_region(address);
        self.vram.write(region, offset, bytes);
    }

    /// ARM9 write to engine A BG VRAM (0x06000000) through the bank mapping.
    pub fn write_bga(&mut self, address: u32, halfword: u16) {
        self.vram.write(Region::BgA, address, &halfword.to_le_bytes());
    }

    /// ARM9 write to engine B BG VRAM (0x06200000) through the bank mapping.
    pub fn write_bgb(&mut self, address: u32, halfword: u16) {
        self.vram.write(Region::BgB, address, &halfword.to_le_bytes());
    }

    /// ARM9 write to engine A OBJ VRAM (0x06400000) through the bank mapping.
    pub fn write_obja(&mut self, address: u32, halfword: u16) {
        self.vram.write(Region::ObjA, address, &halfword.to_le_bytes());
    }

    /// ARM9 write to engine B OBJ VRAM (0x06600000) through the bank mapping.
    pub fn write_objb(&mut self, address: u32, halfword: u16) {
        self.vram.write(Region::ObjB, address, &halfword.to_le_bytes());
    }

    /// ARM9 write to LCDC VRAM (0x06800000, banks mapped to the CPU).
    pub fn write_lcdc(&mut self, address: u32, halfword: u16) {
        self.vram.write(Region::Lcdc, address, &halfword.to_le_bytes());
    }

    /// ARM7 view of banks C/D (0x0600_0000.., 256 KiB mirrored).
    #[inline]
    pub fn read_arm7_u8(&self, address: u32) -> u8 {
        self.vram.read_u8(Region::Arm7, address)
    }

    /// ARM7 16-bit read of VRAM banks C/D mapped to the ARM7 (0x06000000).
    #[inline]
    pub fn read_arm7_u16(&self, address: u32) -> u16 {
        self.vram.read_u16(Region::Arm7, address)
    }

    /// ARM7 32-bit read of VRAM banks C/D (0x06000000).
    #[inline]
    pub fn read_arm7_u32(&self, address: u32) -> u32 {
        self.vram.read_u32(Region::Arm7, address)
    }

    /// ARM7 32-bit write to VRAM banks C/D (0x06000000).
    pub fn write_arm7_u32(&mut self, address: u32, value: u32) {
        self.vram.write(Region::Arm7, address, &value.to_le_bytes());
    }

    /// ARM7 16-bit write to VRAM banks C/D (0x06000000).
    pub fn write_arm7_u16(&mut self, address: u32, value: u16) {
        self.vram.write(Region::Arm7, address, &value.to_le_bytes());
    }

    /// ARM7 8-bit write to VRAM banks C/D (0x06000000).
    pub fn write_arm7_u8(&mut self, address: u32, value: u8) {
        self.vram.write(Region::Arm7, address, &[value]);
    }
}

// DS Memory Maps: https://problemkaputt.de/gbatek.htm#dsmemorymaps
const OAM_MASK: u32 = 0x7FF; // 2K OAM (engine A + engine B)

impl Gpu {
    /// Byte read from OAM (0x07000000, engine A; +0x400 engine B).
    pub fn read_oam_u8(&self, address: u32) -> u8 {
        let index = (address & OAM_MASK) as usize;
        self.oam[index]
    }

    /// Halfword read from OAM.
    pub fn read_oam_u16(&self, address: u32) -> u16 {
        let idx = (address & OAM_MASK) as usize;

        let lo = self.oam[idx];
        let hi = self.oam[(idx + 1) & OAM_MASK as usize];

        u16::from_le_bytes([lo, hi])
    }

    /// Word read from OAM.
    pub fn read_oam_u32(&self, address: u32) -> u32 {
        let idx = (address & OAM_MASK) as usize;

        let b0 = self.oam[idx];
        let b1 = self.oam[(idx + 1) & OAM_MASK as usize];
        let b2 = self.oam[(idx + 2) & OAM_MASK as usize];
        let b3 = self.oam[(idx + 3) & OAM_MASK as usize];

        u32::from_le_bytes([b0, b1, b2, b3])
    }
}

impl Gpu {
    /// Write to palette A (`address` may be absolute; only bits 0-9 count).
    pub fn write_palette_a(&mut self, address: u32, value: u16) {
        let index = (address & 0x3FE) as usize; // halfword-aligned offset in 1K palette RAM
        self.palette_upper[index..index + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// Write to palette B (`address` may be absolute; only bits 0-9 count).
    pub fn write_palette_b(&mut self, address: u32, value: u16) {
        let index = (address & 0x3FE) as usize; // halfword-aligned offset in 1K palette RAM
        self.palette_lower[index..index + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// Halfword write to OAM (0x07000000, 2 KB).
    pub fn write_oam(&mut self, address: u32, halfword: u16) {
        let index = (address & 0x7FE) as usize; // halfword-aligned offset in 2K OAM
        self.oam[index..index + 2].copy_from_slice(&halfword.to_le_bytes());
    }
}
