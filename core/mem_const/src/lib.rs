// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later

// Address helpers

/// Checks if an address is within a given range [start, start + size)
#[inline]
pub const fn addr_in_range(address: u32, start: u32, size: u32) -> bool {
    address >= start && address < start + size
}

// Memory Addresses

/// Start address of main RAM (ARM9)
pub const MAIN_RAM_START: u32 = 0x0200_0000;

/// Start address of shared WRAM between ARM7 and ARM9
pub const SHARED_WRAM_START: u32 = 0x0300_0000;

/// Start address of ARM7 WRAM
pub const ARM7_WRAM_START: u32 = 0x0380_0000;

/// Start address of I/O registers
pub const IO_REGS_START: u32 = 0x0400_0000;

/// Start address of the palette memory
pub const PALETTE_START: u32 = 0x0500_0000;

/// Start address of background VRAM A
pub const VRAM_BGA_START: u32 = 0x0600_0000;

/// Start address of background VRAM B
pub const VRAM_BGB_START: u32 = 0x0620_0000;

/// Start address of VRAM B (C subregion)
pub const VRAM_BGB_C: u32 = 0x0620_0000;

/// Start address of VRAM B (H subregion)
pub const VRAM_BGB_H: u32 = 0x0620_0000;

/// Start address of VRAM B (I subregion)
pub const VRAM_BGB_I: u32 = 0x0620_8000;

/// Start address of object VRAM A
pub const VRAM_OBJA_START: u32 = 0x0640_0000;

/// Start address of object VRAM B
pub const VRAM_OBJB_START: u32 = 0x0660_0000;

/// Start address of LCDC VRAM A
pub const VRAM_LCDC_A: u32 = 0x0680_0000;

/// Start address of LCDC VRAM B
pub const VRAM_LCDC_B: u32 = 0x0682_0000;

/// Start address of LCDC VRAM C
pub const VRAM_LCDC_C: u32 = 0x0684_0000;

/// Start address of LCDC VRAM D
pub const VRAM_LCDC_D: u32 = 0x0686_0000;

/// Start address of LCDC VRAM E
pub const VRAM_LCDC_E: u32 = 0x0688_0000;

/// Start address of LCDC VRAM F
pub const VRAM_LCDC_F: u32 = 0x0689_0000;

/// Start address of LCDC VRAM G
pub const VRAM_LCDC_G: u32 = 0x0689_4000;

/// Start address of LCDC VRAM H
pub const VRAM_LCDC_H: u32 = 0x0689_8000;

/// Start address of LCDC VRAM I
pub const VRAM_LCDC_I: u32 = 0x068A_0000;

/// End address of LCDC VRAM
pub const VRAM_LCDC_END: u32 = 0x068A_4000;

/// Start address of OAM (Object Attribute Memory)
pub const OAM_START: u32 = 0x0700_0000;

/// Start address of GBA ROM memory
pub const GBA_ROM_START: u32 = 0x0800_0000;

/// Start address of GBA RAM memory
pub const GBA_RAM_START: u32 = 0x0A00_0000;

// Memory Sizes

/// Size of VRAM A in bytes: 128KiB
pub const VRAM_A_SIZE: u32 = 1024 * 128;

/// Size of VRAM B in bytes: 128KiB
pub const VRAM_B_SIZE: u32 = 1024 * 128;

/// Size of VRAM C in bytes: 128KiB
pub const VRAM_C_SIZE: u32 = 1024 * 128;

/// Size of VRAM D in bytes: 128KiB
pub const VRAM_D_SIZE: u32 = 1024 * 128;

/// Size of VRAM E in bytes: 64KiB
pub const VRAM_E_SIZE: u32 = 1024 * 64;

/// Size of VRAM F in bytes: 16KiB
pub const VRAM_F_SIZE: u32 = 1024 * 16;

/// Size of VRAM G in bytes: 16KiB
pub const VRAM_G_SIZE: u32 = 1024 * 16;

/// Size of VRAM H in bytes: 32KiB
pub const VRAM_H_SIZE: u32 = 1024 * 32;

/// Size of VRAM I in bytes: 16KiB
pub const VRAM_I_SIZE: u32 = 1024 * 16;

/// Size of ARM9 BIOS in bytes: 4KiB
pub const BIOS9_SIZE: usize = 1024 * 4;

/// Size of ARM7 BIOS in bytes: 16KiB
pub const BIOS7_SIZE: usize = 1024 * 16;

// Masks

/// Mask for ITCM (Instruction Tightly Coupled Memory)
pub const ITCM_MASK: u32 = 0x7FFF;

/// Mask for DTCM (Data Tightly Coupled Memory)
pub const DTCM_MASK: u32 = 0x3FFF;

/// Mask for ARM7 WRAM
pub const ARM7_WRAM_MASK: u32 = 0xFFFF;

/// Mask for main RAM
pub const MAIN_RAM_MASK: u32 = 0x3F_FFFF;

/// Mask for VRAM A
pub const VRAM_A_MASK: u32 = VRAM_A_SIZE - 1;

/// Mask for VRAM B
pub const VRAM_B_MASK: u32 = VRAM_B_SIZE - 1;

/// Mask for VRAM C
pub const VRAM_C_MASK: u32 = VRAM_C_SIZE - 1;

/// Mask for VRAM D
pub const VRAM_D_MASK: u32 = VRAM_D_SIZE - 1;

/// Mask for VRAM E
pub const VRAM_E_MASK: u32 = VRAM_E_SIZE - 1;

/// Mask for VRAM F
pub const VRAM_F_MASK: u32 = VRAM_F_SIZE - 1;

/// Mask for VRAM G
pub const VRAM_G_MASK: u32 = VRAM_G_SIZE - 1;

/// Mask for VRAM H
pub const VRAM_H_MASK: u32 = VRAM_H_SIZE - 1;

/// Mask for VRAM I
pub const VRAM_I_MASK: u32 = VRAM_I_SIZE - 1;

// Other constants

/// Number of pixels per horizontal line
pub const PIXELS_PER_LINE: usize = 256;

/// Number of vertical scanlines
pub const SCANLINES: usize = 192;
