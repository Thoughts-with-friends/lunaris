// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
//! Engine-side and CPU-side VRAM accessors. All of them go through the
//! page-mapped [`crate::vram::Vram`]. BG/OBJ/LCDC/ARM7 accessors take
//! absolute addresses (e.g. `0x0620_0000 + x`; the region wraps them), the
//! 3D and extended palette accessors take offsets inside their slot space.
use crate::gpu_root::Gpu;
use crate::vram::{Region, Vram};

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
    region_readers!(
        Region::BgA,
        read_bga_u8,
        read_bga_u16,
        read_bga_u32,
        read_bga_u64
    );
    region_readers!(
        Region::BgB,
        read_bgb_u8,
        read_bgb_u16,
        read_bgb_u32,
        read_bgb_u64
    );
    region_readers!(
        Region::ObjA,
        read_obja_u8,
        read_obja_u16,
        read_obja_u32,
        read_obja_u64
    );
    region_readers!(
        Region::ObjB,
        read_objb_u8,
        read_objb_u16,
        read_objb_u32,
        read_objb_u64
    );

    /// 3D texture image byte at `address` (offset inside the 512 KiB slots).
    #[inline]
    pub fn read_teximage_u8(&self, address: u32) -> u8 {
        self.vram.read_u8(Region::TexImage, address)
    }

    #[inline]
    pub fn read_teximage_u16(&self, address: u32) -> u16 {
        self.vram.read_u16(Region::TexImage, address)
    }

    /// 3D texture palette halfword at `address` (offset inside the slots).
    #[inline]
    pub fn read_texpal_u16(&self, address: u32) -> u16 {
        self.vram.read_u16(Region::TexPal, address)
    }

    #[inline]
    pub fn read_texpal_u32(&self, address: u32) -> u32 {
        self.vram.read_u32(Region::TexPal, address)
    }

    /// Engine A BG extended palette (`address` = slot * 0x2000 + offset).
    #[inline]
    pub fn read_extpal_bga_u16(&self, address: u32) -> u16 {
        self.vram.read_u16(Region::BgExtPalA, address)
    }

    /// Engine B BG extended palette (`address` = slot * 0x2000 + offset).
    #[inline]
    pub fn read_extpal_bgb_u16(&self, address: u32) -> u16 {
        self.vram.read_u16(Region::BgExtPalB, address)
    }

    /// Engine A OBJ extended palette.
    #[inline]
    pub fn read_extpal_obja(&self, address: u32) -> u16 {
        self.vram.read_u16(Region::ObjExtPalA, address)
    }

    /// Engine B OBJ extended palette.
    #[inline]
    pub fn read_extpal_objb(&self, address: u32) -> u16 {
        self.vram.read_u16(Region::ObjExtPalB, address)
    }

    /// LCDC-mapped VRAM (absolute address in 0x0680_0000..).
    #[inline]
    pub fn read_lcdc_u8(&self, address: u32) -> u8 {
        self.vram.read_u8(Region::Lcdc, address)
    }

    #[inline]
    pub fn read_lcdc_u16(&self, address: u32) -> u16 {
        self.vram.read_u16(Region::Lcdc, address)
    }

    #[inline]
    pub fn read_lcdc_u32(&self, address: u32) -> u32 {
        self.vram.read_u32(Region::Lcdc, address)
    }

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

    pub fn write_bga(&mut self, address: u32, halfword: u16) {
        self.vram
            .write(Region::BgA, address, &halfword.to_le_bytes());
    }

    pub fn write_bgb(&mut self, address: u32, halfword: u16) {
        self.vram
            .write(Region::BgB, address, &halfword.to_le_bytes());
    }

    pub fn write_obja(&mut self, address: u32, halfword: u16) {
        self.vram
            .write(Region::ObjA, address, &halfword.to_le_bytes());
    }

    pub fn write_objb(&mut self, address: u32, halfword: u16) {
        self.vram
            .write(Region::ObjB, address, &halfword.to_le_bytes());
    }

    pub fn write_lcdc(&mut self, address: u32, halfword: u16) {
        self.vram
            .write(Region::Lcdc, address, &halfword.to_le_bytes());
    }

    /// ARM7 view of banks C/D (0x0600_0000.., 256 KiB mirrored).
    #[inline]
    pub fn read_arm7_u8(&self, address: u32) -> u8 {
        self.vram.read_u8(Region::Arm7, address)
    }

    #[inline]
    pub fn read_arm7_u16(&self, address: u32) -> u16 {
        self.vram.read_u16(Region::Arm7, address)
    }

    #[inline]
    pub fn read_arm7_u32(&self, address: u32) -> u32 {
        self.vram.read_u32(Region::Arm7, address)
    }

    pub fn write_arm7_u32(&mut self, address: u32, value: u32) {
        self.vram.write(Region::Arm7, address, &value.to_le_bytes());
    }

    pub fn write_arm7_u16(&mut self, address: u32, value: u16) {
        self.vram.write(Region::Arm7, address, &value.to_le_bytes());
    }

    pub fn write_arm7_u8(&mut self, address: u32, value: u8) {
        self.vram.write(Region::Arm7, address, &[value]);
    }
}
