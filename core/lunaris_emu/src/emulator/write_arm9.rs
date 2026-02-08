use crate::interrupts::Interrupt;

// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
use super::Emulator;
use lunaris_ds_mem_const::*;

impl Emulator {
    /// Writes a 32-bit word to ARM9 memory-mapped address space
    ///
    /// Handles writes to:
    /// - Main RAM
    /// - Shared WRAM (controlled by WRAM CNT)
    /// - GPU registers (BG, WIN, BLDCNT, etc.)
    /// - DMA registers
    /// - IPC, FIFO
    /// - Cartridge AUX SPI registers
    pub fn arm9_write_word(&mut self, address: u32, word: u32) {
        // GPU / DMA / IPC / cartridge / I/O registers
        match address {
            MAIN_RAM_START..SHARED_WRAM_START => {
                // Main RAM
                let index = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[index..index + 4].copy_from_slice(&word.to_le_bytes());
            }
            SHARED_WRAM_START..IO_REGS_START => {
                // Shared WRAM
                let index = match self.wram_cnt {
                    0 => (address & 0x7FFF) as usize,            // Entire 32 KB
                    1 => ((address & 0x3FFF) + 0x4000) as usize, // Second half
                    2 => (address & 0x3FFF) as usize,            // First half
                    3 => return,                                 // Undefined memory
                    _ => unreachable!(),
                };
                self.shared_wram[index..index + 4].copy_from_slice(&word.to_le_bytes());
            }
            0x0400_0000 => self.gpu.set_dispcnt_a(word),
            0x0400_0004 => self.gpu.set_dispstat9((word & 0xFFFF) as u16),
            0x0400_0008 => {
                self.gpu.set_bgcnt_a((word & 0xFFFF) as u16, 0);
                self.gpu.set_bgcnt_a((word >> 16) as u16, 1);
            }
            0x0400_000C => {
                self.gpu.set_bgcnt_a((word & 0xFFFF) as u16, 2);
                self.gpu.set_bgcnt_a((word >> 16) as u16, 3);
            }
            0x0400_000E..=0x0400_0070 => { /* GPU other registers, implement similarly */ }
            0x0400_00B0..=0x0400_00DC => { /* DMA registers, call self.dma.write_* */ }
            0x0400_00E0..=0x0400_00EC => {
                let i = ((address - 0x0400_00E0) / 4) as usize;
                self.dma_fill[i] = word;
            }
            0x0400_0180 => {
                self.ipc_sync_nds9.write(word as u16);
                self.ipc_sync_nds7.receive_input(word as u16);
                if word & (1 << 13) != 0 && self.ipc_sync_nds7.irq_enable {
                    self.request_interrupt7(Interrupt::IpcSync);
                }
            }
            0x0400_0188 => {
                self.fifo7.write_queue(word);
                if self.fifo7.request_nempty_irq {
                    self.request_interrupt7(Interrupt::IpcFifoNempty);
                    self.fifo7.request_nempty_irq = false;
                }
            }
            0x0400_01A0 => {
                self.cart.set_auxspicnt((word & 0xFFFF) as u16);
                self.cart.set_auxspidata(((word >> 16) & 0xFF) as u8);
            }
            0x0400_01A4 => self.cart.set_romctrl(word),
            0x0400_01A8 => {
                self.cart.receive_command((word >> 24) as u8, 3);
                self.cart.receive_command(((word >> 16) & 0xFF) as u8, 2);
                self.cart.receive_command(((word >> 8) & 0xFF) as u8, 1);
                self.cart.receive_command((word & 0xFF) as u8, 0);
            }
            0x0400_01AC => {
                self.cart.receive_command((word >> 24) as u8, 7);
                self.cart.receive_command(((word >> 16) & 0xFF) as u8, 6);
                self.cart.receive_command(((word >> 8) & 0xFF) as u8, 5);
                self.cart.receive_command((word & 0xFF) as u8, 4);
            }
            0x0400_0208 => self.int9_reg.ime = word & 0x1,
            0x0400_0210 => self.int9_reg.irq_enable = word,
            0x0400_0214 => {
                self.int9_reg.irq_flags &= !word;
                self.gpu.check_gxfifo_irq();
            }
            0x0400_0240 => {
                self.gpu.set_vramcnt_a((word & 0xFF) as u8);
                self.gpu.set_vramcnt_b(((word >> 8) & 0xFF) as u8);
                self.gpu.set_vramcnt_c(((word >> 16) & 0xFF) as u8);
                self.gpu.set_vramcnt_d((word >> 24) as u8);
            }
            0x0400_0290..=0x0400_029C => { /* Division registers, implement start_division */ }
            0x0400_02B8..=0x0400_02BC => { /* Square root registers, implement start_sqrt */ }
            0x0400_0304 => self.gpu.set_powcnt1((word & 0xFFFF) as u16),
            0x0400_0350 => self.gpu.set_clear_color(word),
            0x0400_0600 => self.gpu.set_gxstat(word),
            0x0400_1000 => self.gpu.set_dispcnt_b(word),
            0x0400_0330..0x0400_0340 | 0x0400_0360..0x0400_0380 => {} // FOG_TABLE / EDGE_COLOR (TODO)
            0x0400_0380..0x0400_03C0 => {
                self.gpu
                    .set_toon_table((address & 0x3F) >> 1, (word & 0xFFFF) as u16);
                self.gpu
                    .set_toon_table(((address + 2) & 0x3F) >> 1, (word >> 16) as u16);
            }
            0x0400_0400..0x0400_0440 => self.gpu.write_gxfifo(word),
            0x0400_0440..0x0400_05CC => self.gpu.write_fifo_direct(address, word),
            PALETTE_START..VRAM_BGA_START => {
                if (address & 0x7FF) < 0x400 {
                    self.gpu.write_palette_a(address, (word & 0xFFFF) as u16);
                    self.gpu.write_palette_a(address + 2, (word >> 16) as u16);
                } else {
                    self.gpu.write_palette_b(address, (word & 0xFFFF) as u16);
                    self.gpu.write_palette_b(address + 2, (word >> 16) as u16);
                }
            }
            VRAM_BGA_START..VRAM_BGB_START => {
                self.gpu.write_bga(address, (word & 0xFFFF) as u16);
                self.gpu.write_bga(address + 2, (word >> 16) as u16);
            }
            VRAM_BGB_START..VRAM_OBJA_START => {
                self.gpu.write_bgb(address, (word & 0xFFFF) as u16);
                self.gpu.write_bgb(address + 2, (word >> 16) as u16);
            }
            VRAM_OBJA_START..VRAM_OBJB_START => {
                self.gpu.write_obja(address, (word & 0xFFFF) as u16);
                self.gpu.write_obja(address + 2, (word >> 16) as u16);
            }

            VRAM_OBJB_START..VRAM_LCDC_A => {
                self.gpu.write_objb(address, (word & 0xFFFF) as u16);
                self.gpu.write_objb(address + 2, (word >> 16) as u16);
            }

            OAM_START..GBA_ROM_START => {
                self.gpu.write_oam(address, (word & 0xFFFF) as u16);
                self.gpu.write_oam(address + 2, (word >> 16) as u16);
            }
            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!("(9) Unrecognized word write of ${word:08X} to ${address:08X}");
            }
        }
    }

    /// Writes a 16-bit halfword to the specified ARM9 memory address.
    ///
    /// This function emulates the ARM9 memory map of the Nintendo DS, handling:
    /// - Main RAM and shared WRAM writes
    /// - Palette, VRAM, OAM, and LCDC memory writes
    /// - IO registers (timers, DMA, IPC, GPU, division/sqrt, etc.)
    ///
    /// Unrecognized addresses are logged.
    ///
    /// # Parameters
    /// - `address`: 32-bit ARM9 memory address to write to.
    /// - `halfword`: 16-bit value to write.
    pub fn arm9_write_halfword(&mut self, address: u32, halfword: u16) {
        // IO registers
        match address {
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let idx = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[idx..idx + 2].copy_from_slice(&halfword.to_le_bytes());
            }

            // Palette memory
            PALETTE_START..VRAM_BGA_START => match address & 0x7FF {
                ..0x400 => self.gpu.write_palette_a(address, halfword),
                _ => self.gpu.write_palette_b(address, halfword),
            },

            // LCDC VRAM
            VRAM_LCDC_A..OAM_START => {
                self.gpu.write_lcdc(address, halfword);
            }

            // Background VRAM A
            VRAM_BGA_START..VRAM_BGB_START => {
                self.gpu.write_bga(address, halfword);
            }

            // Background VRAM B
            VRAM_BGB_START..VRAM_OBJA_START => {
                self.gpu.write_bgb(address, halfword);
            }

            // Object VRAM A
            VRAM_OBJA_START..VRAM_OBJB_START => {
                self.gpu.write_obja(address, halfword);
            }

            // Object VRAM B
            VRAM_OBJB_START..VRAM_LCDC_A => {
                self.gpu.write_objb(address, halfword);
            }

            // OAM
            OAM_START..GBA_ROM_START => {
                self.gpu.write_oam(address, halfword);
            }

            // Shared WRAM
            SHARED_WRAM_START..IO_REGS_START => {
                match self.wram_cnt {
                    0 => {
                        let idx = (address & 0x7FFF) as usize;
                        self.shared_wram[idx..idx + 2].copy_from_slice(&halfword.to_le_bytes());
                    }
                    1 => {
                        let idx = ((address & 0x3FFF) + 0x4000) as usize;
                        self.shared_wram[idx..idx + 2].copy_from_slice(&halfword.to_le_bytes());
                    }
                    2 => {
                        let idx = (address & 0x3FFF) as usize;
                        self.shared_wram[idx..idx + 2].copy_from_slice(&halfword.to_le_bytes());
                    }
                    _ => {} // Undefined memory, do nothing
                }
            }
            0x04000000 => self.gpu.set_dispcnt_a(halfword as u32),
            0x04000004 => self.gpu.set_dispstat9(halfword),
            0x04000008 => self.gpu.set_bgcnt_a(halfword, 0),
            0x0400000A => self.gpu.set_bgcnt_a(halfword, 1),
            0x0400000C => self.gpu.set_bgcnt_a(halfword, 2),
            0x0400000E => self.gpu.set_bgcnt_a(halfword, 3),
            0x04000010 => self.gpu.set_bghofs_a(halfword, 0),
            0x04000012 => self.gpu.set_bgvofs_a(halfword, 0),
            0x04000014 => self.gpu.set_bghofs_a(halfword, 1),
            0x04000016 => self.gpu.set_bgvofs_a(halfword, 1),
            0x04000018 => self.gpu.set_bghofs_a(halfword, 2),
            0x0400001A => self.gpu.set_bgvofs_a(halfword, 2),
            0x0400001C => self.gpu.set_bghofs_a(halfword, 3),
            0x0400001E => self.gpu.set_bgvofs_a(halfword, 3),
            0x04000020 => self.gpu.set_bg2p_a(halfword, 0),
            0x04000022 => self.gpu.set_bg2p_a(halfword, 1),
            0x04000024 => self.gpu.set_bg2p_a(halfword, 2),
            0x04000026 => self.gpu.set_bg2p_a(halfword, 3),
            0x04000030 => self.gpu.set_bg3p_a(halfword, 0),
            0x04000032 => self.gpu.set_bg3p_a(halfword, 1),
            0x04000034 => self.gpu.set_bg3p_a(halfword, 2),
            0x04000036 => self.gpu.set_bg3p_a(halfword, 3),
            0x04000040 => self.gpu.set_win0h_a(halfword),
            0x04000042 => self.gpu.set_win1h_a(halfword),
            0x04000044 => self.gpu.set_win0v_a(halfword),
            0x04000046 => self.gpu.set_win1v_a(halfword),
            0x04000048 => self.gpu.set_winin_a(halfword),
            0x0400004A => self.gpu.set_winout_a(halfword),
            0x0400004C => self.gpu.set_mosaic_a(halfword),
            0x04000050 => self.gpu.set_bldcnt_a(halfword),
            0x04000052 => self.gpu.set_bldalpha_a(halfword),
            0x04000054 => self.gpu.set_bldy_a(halfword as u8),
            0x04000060 => self.gpu.set_disp3dcnt(halfword),
            0x0400006C => self.gpu.set_master_bright_a(halfword),
            0x040000BA => self.dma.write_cnt(0, halfword),
            0x040000C6 => self.dma.write_cnt(1, halfword),
            0x040000D0 => self.dma.write_len(2, halfword),
            0x040000D2 => self.dma.write_cnt(2, halfword),
            0x040000DE => self.dma.write_cnt(3, halfword),
            0x04000100 => self.nds_timing.write_lo(halfword, 4),
            0x04000102 => self.nds_timing.write_hi(halfword, 4),
            0x04000104 => self.nds_timing.write_lo(halfword, 5),
            0x04000106 => self.nds_timing.write_hi(halfword, 5),
            0x04000108 => self.nds_timing.write_lo(halfword, 6),
            0x0400010A => self.nds_timing.write_hi(halfword, 6),
            0x0400010C => self.nds_timing.write_lo(halfword, 7),
            0x0400010E => self.nds_timing.write_hi(halfword, 7),
            0x04000180 => {
                self.ipc_sync_nds9.write(halfword);
                self.ipc_sync_nds7.receive_input(halfword);
                if (halfword & (1 << 13) != 0) && self.ipc_sync_nds7.irq_enable {
                    self.request_interrupt7(Interrupt::IpcSync);
                }
            }
            0x04000184 => {
                self.fifo9.write_cnt(halfword);
                if self.fifo9.request_empty_irq {
                    self.request_interrupt9(Interrupt::IpcFifoEmpty);
                    self.fifo9.request_empty_irq = false;
                }
                if self.fifo9.request_nempty_irq {
                    self.request_interrupt9(Interrupt::IpcFifoNempty);
                    self.fifo9.request_nempty_irq = false;
                }
            }
            0x040001A0 => self.cart.set_auxspicnt(halfword),
            0x04000204 => self.ex_mem_cnt = halfword,
            0x04000208 => self.int9_reg.ime = (halfword & 0x1) as u32,
            0x04000248 => {
                self.gpu.set_vramcnt_h(halfword as u8);
                self.gpu.set_vramcnt_i((halfword >> 8) as u8);
            }
            0x04000280 => {
                self.divcnt = halfword;
                self.start_division();
            }
            0x040002B0 => {
                self.sqrtcnt = halfword;
                self.start_sqrt();
            }
            0x04000300 => self.postflg9 = (halfword & 0x1) as u8,
            0x04000304 => self.gpu.set_powcnt1(halfword),
            0x04000340 => {} // TODO: ALPHA_TEST_REF
            0x04000354 => self.gpu.set_clear_depth(halfword as u32),
            0x04000356 => {} // TODO: CLRIMAGE_OFFSET
            0x0400035C => {} // TODO: FOG_OFFSET
            0x04001000 => self.gpu.set_dispcnt_b(halfword as u32),
            0x04001008 => self.gpu.set_bgcnt_b(halfword, 0),
            0x0400100A => self.gpu.set_bgcnt_b(halfword, 1),
            0x0400100C => self.gpu.set_bgcnt_b(halfword, 2),
            0x0400100E => self.gpu.set_bgcnt_b(halfword, 3),
            0x04001010 => self.gpu.set_bghofs_b(halfword, 0),
            0x04001012 => self.gpu.set_bgvofs_b(halfword, 0),
            0x04001014 => self.gpu.set_bghofs_b(halfword, 1),
            0x04001016 => self.gpu.set_bgvofs_b(halfword, 1),
            0x04001018 => self.gpu.set_bghofs_b(halfword, 2),
            0x0400101A => self.gpu.set_bgvofs_b(halfword, 2),
            0x0400101C => self.gpu.set_bghofs_b(halfword, 3),
            0x0400101E => self.gpu.set_bgvofs_b(halfword, 3),
            0x04001020 => self.gpu.set_bg2p_b(halfword, 0),
            0x04001022 => self.gpu.set_bg2p_b(halfword, 1),
            0x04001024 => self.gpu.set_bg2p_b(halfword, 2),
            0x04001026 => self.gpu.set_bg2p_b(halfword, 3),
            0x04001030 => self.gpu.set_bg3p_b(halfword, 0),
            0x04001032 => self.gpu.set_bg3p_b(halfword, 1),
            0x04001034 => self.gpu.set_bg3p_b(halfword, 2),
            0x04001036 => self.gpu.set_bg3p_b(halfword, 3),
            0x04001040 => self.gpu.set_win0h_b(halfword),
            0x04001042 => self.gpu.set_win1h_b(halfword),
            0x04001044 => self.gpu.set_win0v_b(halfword),
            0x04001046 => self.gpu.set_win1v_b(halfword),
            0x04001048 => self.gpu.set_winin_b(halfword),
            0x0400104A => self.gpu.set_winout_b(halfword),
            0x0400104C => self.gpu.set_mosaic_b(halfword),
            0x04001050 => self.gpu.set_bldcnt_b(halfword),
            0x04001052 => self.gpu.set_bldalpha_b(halfword),
            0x04001054 => self.gpu.set_bldy_b(halfword as u8),
            0x0400106C => self.gpu.set_master_bright_b(halfword),
            0x04000330..0x04000340 => {} // EDGE_COLOR region (0x04000330 - 0x04000340)
            0x04000380..0x040003C0 => {
                // TOON table
                self.gpu.set_toon_table((address & 0x3F) >> 1, halfword);
            }
            GBA_ROM_START.. => {} // GBA ROM
            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!(
                    "\n(9) Unrecognized halfword write of ${halfword:04X} to ${address:08X}"
                );
            }
        }
    }

    /// Writes an 8-bit byte to the specified ARM9 memory address.
    ///
    /// This function emulates the ARM9 memory map of the Nintendo DS for byte writes,
    /// handling:
    /// - Main RAM writes
    /// - Certain GPU and cart registers
    /// - Warning for unsupported 8-bit VRAM writes
    ///
    /// Unrecognized addresses are logged.
    ///
    /// # Parameters
    /// - `address`: 32-bit ARM9 memory address to write to.
    /// - `byte`: 8-bit value to write.
    pub fn arm9_write_byte(&mut self, address: u32, byte: u8) {
        // IO registers / special addresses
        match address {
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let idx = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[idx] = byte;
            }
            PALETTE_START..VRAM_BGA_START => {} // Palette memory (ignored for byte writes)
            0x0400004C => self.gpu.set_mosaic_a(byte as u16),
            0x040001A1 => self.cart.set_hi_auxspicnt(byte),
            0x040001A2 => {
                println!("\nAUXSPIDATA: ${:02X}", byte);
                self.cart.set_auxspidata(byte);
            }
            0x040001A8..=0x040001AF => {
                self.cart
                    .receive_command(byte, (address - 0x040001A8) as usize);
            }
            0x04000208 => self.int9_reg.ime = (byte & 0x1) as u32,
            0x04000240 => self.gpu.set_vramcnt_a(byte),
            0x04000241 => self.gpu.set_vramcnt_b(byte),
            0x04000242 => self.gpu.set_vramcnt_c(byte),
            0x04000243 => self.gpu.set_vramcnt_d(byte),
            0x04000244 => self.gpu.set_vramcnt_e(byte),
            0x04000245 => self.gpu.set_vramcnt_f(byte),
            0x04000246 => self.gpu.set_vramcnt_g(byte),
            0x04000247 => self.wram_cnt = byte & 0x3,
            0x04000248 => self.gpu.set_vramcnt_h(byte),
            0x04000249 => self.gpu.set_vramcnt_i(byte),
            0x0400104C => self.gpu.set_mosaic_b(byte as u16),
            0x04001054 => self.gpu.set_bldy_b(byte),
            PALETTE_START..GBA_ROM_START => {
                #[cfg(feature = "tracing")]
                tracing::warn!("\nWarning: 8-bit write to VRAM ${address:08X}");
            }
            _ => {
                // Unrecognized byte write
                #[cfg(feature = "tracing")]
                tracing::warn!("\n(9) Unrecognized byte write of ${byte:02X} to ${address:08X}");
            }
        }
    }
}
