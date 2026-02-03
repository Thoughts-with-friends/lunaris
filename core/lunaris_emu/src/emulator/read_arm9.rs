// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
use super::Emulator;
use crate::interrupts::Interrupt;
use lunaris_ds_mem_const::*;

impl Emulator {
    pub fn arm9_read_word(&mut self, address: u32) -> u32 {
        match address {
            // ARM9 BIOS
            0xFFFF_0000..=0xFFFF_FFFF => {
                let offset = (address - 0xFFFF_0000) as usize;
                u32::from_le_bytes(self.arm9_bios[offset..offset + 4].try_into().unwrap())
            }

            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let offset = (address & MAIN_RAM_MASK) as usize;
                u32::from_le_bytes(self.main_ram[offset..offset + 4].try_into().unwrap())
            }

            // Shared WRAM
            SHARED_WRAM_START..IO_REGS_START => {
                let offset = match self.wram_cnt {
                    0 => (address & 0x7FFF) as usize,            // Entire 32 KB
                    1 => ((address & 0x3FFF) + 0x4000) as usize, // Second half
                    2 => (address & 0x3FFF) as usize,            // First half
                    3 => return 0,                               // Undefined memory
                    _ => unreachable!(),
                };
                u32::from_le_bytes(self.shared_wram[offset..offset + 4].try_into().unwrap())
            }

            0x04000000 => self.gpu.get_dispcnt_a(),
            0x04000010 => {
                self.gpu.get_bghofs_a(0) as u32 | ((self.gpu.get_bgvofs_a(0) as u32) << 16)
            }
            0x04000014 => {
                self.gpu.get_bghofs_a(1) as u32 | ((self.gpu.get_bgvofs_a(1) as u32) << 16)
            }
            0x04000018 => {
                self.gpu.get_bghofs_a(2) as u32 | ((self.gpu.get_bgvofs_a(2) as u32) << 16)
            }
            0x0400001C => {
                self.gpu.get_bghofs_a(3) as u32 | ((self.gpu.get_bgvofs_a(3) as u32) << 16)
            }
            0x04000064 => self.gpu.get_dispcapcnt_a(),
            0x040000B0 => self.dma.read_source(0),
            0x040000B8 => self.dma.read_len(0) as u32 | ((self.dma.read_cnt(0) as u32) << 16),
            0x040000C4 => self.dma.read_len(1) as u32 | ((self.dma.read_cnt(1) as u32) << 16),
            0x040000D0 => self.dma.read_len(2) as u32 | ((self.dma.read_cnt(2) as u32) << 16),
            0x040000DC => self.dma.read_len(3) as u32 | ((self.dma.read_cnt(3) as u32) << 16),
            0x040000E0 => self.dma_fill[0],
            0x040000E4 => self.dma_fill[1],
            0x040000E8 => self.dma_fill[2],
            0x040000EC => self.dma_fill[3],
            0x04000180 => self.ipc_sync_nds9.read().into(),
            0x040001A4 => self.cart.get_romctrl(),
            0x04000208 => self.int9_reg.ime,
            0x04000210 => self.int9_reg.irq_enable,
            0x04000214 => self.int9_reg.irq_flags,
            0x04000240 => self.gpu.get_vramcnt_a() as u32,
            0x04000290 => (self.div_numer & 0xFFFF_FFFF) as u32,
            0x04000294 => (self.div_numer >> 32) as u32,
            0x04000298 => (self.div_denom & 0xFFFF_FFFF) as u32,
            0x0400029C => (self.div_denom >> 32) as u32,
            0x040002A0 => (self.div_result & 0xFFFF_FFFF) as u32,
            0x040002A4 => (self.div_result >> 32) as u32,
            0x040002A8 => (self.div_remresult & 0xFFFF_FFFF) as u32,
            0x040002AC => (self.div_remresult >> 32) as u32,
            0x040002B4 => self.sqrt_result,
            0x040002B8 => (self.sqrt_param & 0xFFFF_FFFF) as u32,
            0x040002BC => (self.sqrt_param >> 32) as u32,
            0x04000600 => self.gpu.get_gxstat(),
            0x04000604 => {
                self.gpu.get_poly_count() as u32 | ((self.gpu.get_vert_count() as u32) << 16)
            }
            0x04001000 => self.gpu.get_dispcnt_b(),
            0x04001010 => {
                self.gpu.get_bghofs_b(0) as u32 | ((self.gpu.get_bgvofs_b(0) as u32) << 16)
            }
            0x04001014 => {
                self.gpu.get_bghofs_b(1) as u32 | ((self.gpu.get_bgvofs_b(1) as u32) << 16)
            }
            0x04001018 => {
                self.gpu.get_bghofs_b(2) as u32 | ((self.gpu.get_bgvofs_b(2) as u32) << 16)
            }
            0x0400101c => {
                self.gpu.get_bghofs_b(3) as u32 | ((self.gpu.get_bgvofs_b(3) as u32) << 16)
            }
            0x04004000 | 0x04004008 => 0,
            0x04100000 => {
                let word = self.fifo7.read_queue();
                if self.fifo7.request_empty_irq {
                    self.request_interrupt7(Interrupt::IpcFifoEmpty);
                    self.fifo7.request_empty_irq = false;
                }
                word
            }
            0x04100010 => self.cart.get_output(),
            PALETTE_START..VRAM_BGA_START => {
                if (address & 0x7FF) < 0x400 {
                    let lo = self.gpu.read_palette_a(address) as u32;
                    let hi = self.gpu.read_palette_a(address + 2) as u32;
                    lo | (hi << 16)
                } else {
                    let lo = self.gpu.read_palette_b(address) as u32;
                    let hi = self.gpu.read_palette_b(address + 2) as u32;
                    lo | (hi << 16)
                }
            }
            0x04000640..0x04000680 => self.gpu.read_clip_mtx(address),
            0x04000680..0x040006A4 => self.gpu.read_vec_mtx(address),
            VRAM_BGA_START..VRAM_BGB_START => self.gpu.read_bga::<u32>(address),
            VRAM_BGB_START..VRAM_OBJA_START => self.gpu.read_bgb::<u32>(address),
            VRAM_LCDC_A..VRAM_LCDC_END => self.gpu.read_lcdc::<u32>(address),
            OAM_START..GBA_ROM_START => self.gpu.read_oam_u32(address),
            _ => {
                if address >= GBA_ROM_START {
                    0xFFFF_FFFF
                } else {
                    #[cfg(feature = "tracing")]
                    tracing::warn!("ARM9: Unrecognized word read from ${address:08X}");
                    0
                }
            }
        }
    }

    /// Reads a 16-bit halfword from ARM9 memory-mapped address space
    ///
    /// This function handles reads from:
    /// - ARM9 BIOS
    /// - Main RAM
    /// - Shared WRAM (with WRAM_CNT control)
    /// - Palette memory
    /// - VRAM for OBJ and LCDC
    /// - I/O registers (GPU, DMA, timers, input, etc.)
    /// - Cartridge SPI and ROM
    pub fn arm9_read_halfword(&self, address: u32) -> u16 {
        // I/O registers
        match address {
            0xFFFF_0000.. => {
                // ARM9 BIOS
                let offset = (address - 0xFFFF_0000) as usize;
                u16::from_le_bytes(self.arm9_bios[offset..offset + 2].try_into().unwrap())
            }
            MAIN_RAM_START..SHARED_WRAM_START => {
                // Main RAM
                let offset = (address & MAIN_RAM_MASK) as usize;
                u16::from_le_bytes(self.main_ram[offset..offset + 2].try_into().unwrap())
            }
            SHARED_WRAM_START..IO_REGS_START => {
                // Shared WRAM
                let slice = match self.wram_cnt {
                    0 => &self.shared_wram[(address & 0x7FFF) as usize..][..2], // Entire 32 KB
                    1 => &self.shared_wram[((address & 0x3FFF) + 0x4000) as usize..][..2], // Second half
                    2 => &self.shared_wram[(address & 0x3FFF) as usize..][..2], // First half
                    3 => return 0,                                              // Undefined memory
                    _ => unreachable!(),
                };
                u16::from_le_bytes(slice.try_into().unwrap())
            }
            PALETTE_START..VRAM_BGA_START => {
                // Palette memory
                if (address & 0x7FF) < 0x400 {
                    self.gpu.read_palette_a(address)
                } else {
                    self.gpu.read_palette_b(address)
                }
            }
            VRAM_OBJA_START..VRAM_OBJB_START => self.gpu.read_obja::<u16>(address), // VRAM OBJ A/B
            VRAM_OBJB_START..VRAM_LCDC_A => self.gpu.read_objb::<u16>(address),
            VRAM_LCDC_A..VRAM_LCDC_END => self.gpu.read_lcdc::<u16>(address), // VRAM LCDC
            0x0400_0000 => self.gpu.get_dispcnt_a() as u16,
            0x0400_0004 => self.gpu.get_dispstat9(),
            0x0400_0006 => self.gpu.get_vcount(),
            0x0400_0008 => self.gpu.get_bgcnt_a(0),
            0x0400_000a => self.gpu.get_bgcnt_a(1),
            0x0400_000c => self.gpu.get_bgcnt_a(2),
            0x0400_000e => self.gpu.get_bgcnt_a(3),
            0x0400_0044 => self.gpu.get_win0v_a(),
            0x0400_0046 => self.gpu.get_win1v_a(),
            0x0400_0048 => self.gpu.get_winin_a(),
            0x0400_004a => self.gpu.get_winout_a(),
            0x0400_0050 => self.gpu.get_bldcnt_a(),
            0x0400_0052 => self.gpu.get_bldalpha_a(),
            0x0400_0060 => self.gpu.get_disp3dcnt(),
            0x0400_006c => self.gpu.get_master_bright_a(),
            0x0400_00ba => self.dma.read_cnt(0),
            0x0400_00c6 => self.dma.read_cnt(1),
            0x0400_00d2 => self.dma.read_cnt(2),
            0x0400_00de => self.dma.read_cnt(3),
            0x0400_00E0 => (self.dma_fill[0] & 0xFFFF) as u16,
            0x0400_00E4 => (self.dma_fill[1] & 0xFFFF) as u16,
            0x0400_00E8 => (self.dma_fill[2] & 0xFFFF) as u16,
            0x0400_00EC => (self.dma_fill[3] & 0xFFFF) as u16,
            0x0400_0100 => self.timers.read_lo(4),
            0x0400_0104 => self.timers.read_lo(5),
            0x0400_0108 => self.timers.read_lo(6),
            0x0400_010C => self.timers.read_lo(7),
            0x0400_0130 => self.key_input.get_value(),
            0x0400_0180 => self.ipc_sync_nds9.read(),
            0x0400_0184 => self.fifo9.read_cnt(),
            0x0400_01A0 => self.cart.get_auxspicnt(),
            0x0400_0204 => self.ex_mem_cnt,
            0x0400_0208 => self.int9_reg.ime as u16,
            0x0400_0280 => self.divcnt,
            0x0400_02B0 => self.sqrtcnt,
            0x0400_0300 => self.postflg9.into(),
            0x0400_0304 => self.gpu.get_powcnt1(),
            0x0400_0604 => self.gpu.get_poly_count(),
            0x0400_0606 => self.gpu.get_vert_count(),
            0x0400_1000 => (self.gpu.get_dispcnt_b() & 0xffff) as u16,
            0x0400_1008 => self.gpu.get_bgcnt_b(0),
            0x0400_100a => self.gpu.get_bgcnt_b(1),
            0x0400_100c => self.gpu.get_bgcnt_b(2),
            0x0400_100e => self.gpu.get_bgcnt_b(3),
            0x0400_1044 => self.gpu.get_win0v_b(),
            0x0400_1046 => self.gpu.get_win1v_b(),
            0x0400_1048 => self.gpu.get_winin_b(),
            0x0400_104a => self.gpu.get_winout_b(),
            0x0400_1050 => self.gpu.get_bldcnt_b(),
            0x0400_1052 => self.gpu.get_bldalpha_b(),
            0x0400_106c => self.gpu.get_master_bright_b(),
            0x0400_0630..0x0400_0636 => self.gpu.read_vec_test(address),
            _ => {
                if (VRAM_BGA_START..VRAM_BGB_START).contains(&address) {
                    self.gpu.read_bga::<u16>(address)
                } else if (VRAM_BGB_START..VRAM_OBJA_START).contains(&address) {
                    self.gpu.read_bgb::<u16>(address)
                } else if address >= GBA_ROM_START {
                    0xFFFF
                } else {
                    #[cfg(feature = "tracing")]
                    tracing::warn!("(9) Unrecognized halfword read from ${:08X}", address);
                    0
                }
            }
        }
    }

    /// Reads a single byte from ARM9 memory-mapped address space
    ///
    /// Handles reads from:
    /// - Main RAM
    /// - Shared WRAM (controlled by WRAM CNT)
    /// - Palette and VRAM memory
    /// - LCDC, OAM
    /// - Cartridge SPI registers
    /// - Some I/O registers
    pub fn arm9_read_byte(&self, address: u32) -> u8 {
        // I/O registers
        match address {
            MAIN_RAM_START..SHARED_WRAM_START => self.main_ram[(address & MAIN_RAM_MASK) as usize], // Main RAM
            SHARED_WRAM_START..IO_REGS_START => {
                // Shared WRAM

                match self.wram_cnt {
                    0 => self.shared_wram[(address & 0x7FFF) as usize], // Entire 32 KB
                    1 => self.shared_wram[((address & 0x3FFF) + 0x4000) as usize], // Second half
                    2 => self.shared_wram[(address & 0x3FFF) as usize], // First half
                    3 => 0,                                             // Undefined memory
                    _ => unreachable!(),
                }
            }
            0x0400_01a2 => self.cart.read_auxspidata(),
            0x0400_01a8..=0x0400_01af => self.cart.read_command((address & 0x7) as usize),
            0x0400_0208 => self.int9_reg.ime as u8,
            0x0400_0240 => self.gpu.get_vramcnt_a(),
            0x0400_0241 => self.gpu.get_vramcnt_b(),
            0x0400_0247 => self.wram_cnt & 0x3,
            0x0400_0300 => self.postflg9,
            0x0400_4000 => 0,
            PALETTE_START..VRAM_BGA_START => {
                if (address & 0x7FF) < 0x400 {
                    (self.gpu.read_palette_a(address) & 0xFF) as u8
                } else {
                    (self.gpu.read_palette_b(address) & 0xFF) as u8
                }
            }
            VRAM_BGA_START..VRAM_BGB_START => self.gpu.read_bga::<u8>(address),
            VRAM_BGB_START..VRAM_OBJA_START => self.gpu.read_bgb::<u8>(address),
            VRAM_LCDC_A..OAM_START => self.gpu.read_lcdc::<u8>(address), // VRAM LCDC
            OAM_START..GBA_ROM_START => self.gpu.read_oam_u8(address),   // OAM
            GBA_ROM_START.. => 0xFF,                                     // GBA ROM
            _ => {
                // Palette memory
                #[cfg(feature = "tracing")]
                tracing::warn!("(9) Unrecognized byte read from ${:08X}", address);
                0
            }
        }
    }
}
