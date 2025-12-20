// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
impl Emulator {
    /// Writes a 32-bit word to ARM9 memory-mapped address space
    ///
    /// Handles writes to:
    /// - Main RAM
    /// - Shared WRAM (controlled by WRAMCNT)
    /// - GPU registers (BG, WIN, BLDCNT, etc.)
    /// - DMA registers
    /// - IPC, FIFO
    /// - Cartridge AUX SPI registers
    pub fn arm9_write_word(&mut self, address: u32, word: u32) {
        // Main RAM
        if address >= MAIN_RAM_START && address < SHARED_WRAM_START {
            let index = (address & MAIN_RAM_MASK) as usize;
            self.main_ram[index..index + 4].copy_from_slice(&word.to_le_bytes());
            return;
        }

        // Shared WRAM
        if address >= SHARED_WRAM_START && address < IO_REGS_START {
            let index = match self.wramcnt {
                0 => (address & 0x7FFF) as usize,            // Entire 32 KB
                1 => ((address & 0x3FFF) + 0x4000) as usize, // Second half
                2 => (address & 0x3FFF) as usize,            // First half
                3 => return,                                 // Undefined memory
                _ => unreachable!(),
            };
            self.shared_wram[index..index + 4].copy_from_slice(&word.to_le_bytes());
            return;
        }

        // GPU / DMA / IPC / cartridge / I/O registers
        match address {
            0x0400_0000 => self.gpu.set_DISPCNT_A(word),
            0x0400_0004 => self.gpu.set_DISPSTAT9((word & 0xFFFF) as u16),
            0x0400_0008 => {
                self.gpu.set_BGCNT_A((word & 0xFFFF) as u16, 0);
                self.gpu.set_BGCNT_A((word >> 16) as u16, 1);
            }
            0x0400_000C => {
                self.gpu.set_BGCNT_A((word & 0xFFFF) as u16, 2);
                self.gpu.set_BGCNT_A((word >> 16) as u16, 3);
            }
            0x0400_000E..=0x0400_0070 => { /* GPU other registers, implement similarly */ }
            0x0400_00B0..=0x0400_00DC => { /* DMA registers, call self.dma.write_* */ }
            0x0400_00E0..=0x0400_00EC => {
                let i = ((address - 0x0400_00E0) / 4) as usize;
                self.dmafill[i] = word;
            }
            0x0400_0180 => {
                self.ipcsync_nds9.write(word);
                self.ipcsync_nds7.receive_input(word);
                if word & (1 << 13) != 0 && self.ipcsync_nds7.irq_enable {
                    self.request_interrupt7(INTERRUPT::IPCSYNC);
                }
            }
            0x0400_0188 => {
                self.fifo7.write_queue(word);
                if self.fifo7.request_nempty_irq {
                    self.request_interrupt7(INTERRUPT::IPC_FIFO_NEMPTY);
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
            0x0400_0210 => self.int9_reg.ie = word,
            0x0400_0214 => {
                self.int9_reg.if_ &= !word;
                self.gpu.check_gxfifo_irq();
            }
            0x0400_0240 => {
                self.gpu.set_VRAMCNT_A((word & 0xFF) as u8);
                self.gpu.set_VRAMCNT_B(((word >> 8) & 0xFF) as u8);
                self.gpu.set_VRAMCNT_C(((word >> 16) & 0xFF) as u8);
                self.gpu.set_VRAMCNT_D((word >> 24) as u8);
            }
            0x0400_0290..=0x0400_029C => { /* Division registers, implement start_division */ }
            0x0400_02B8..=0x0400_02BC => { /* Square root registers, implement start_sqrt */ }
            0x0400_0304 => self.gpu.set_POWCNT1((word & 0xFFFF) as u16),
            0x0400_0350 => self.gpu.set_CLEAR_COLOR(word),
            0x0400_0600 => self.gpu.set_GXSTAT(word),
            0x0400_1000 => self.gpu.set_DISPCNT_B(word),
            _ => {
                if (0x0400_0330..0x0400_0340).contains(&address)
                    || (0x0400_0360..0x0400_0380).contains(&address)
                {
                    return; // FOG_TABLE / EDGE_COLOR (TODO)
                } else if (0x0400_0380..0x0400_03C0).contains(&address) {
                    self.gpu
                        .set_TOON_TABLE(((address & 0x3F) >> 1) as usize, (word & 0xFFFF) as u16);
                    self.gpu.set_TOON_TABLE(
                        (((address + 2) & 0x3F) >> 1) as usize,
                        (word >> 16) as u16,
                    );
                    return;
                } else if (0x0400_0400..0x0400_0440).contains(&address) {
                    self.gpu.write_GXFIFO(word);
                    return;
                } else if (0x0400_0440..0x0400_05CC).contains(&address) {
                    self.gpu.write_FIFO_direct(address, word);
                    return;
                } else if address >= PALETTE_START && address < VRAM_BGA_START {
                    if (address & 0x7FF) < 0x400 {
                        self.gpu.write_palette_A(address, (word & 0xFFFF) as u16);
                        self.gpu.write_palette_A(address + 2, (word >> 16) as u16);
                    } else {
                        self.gpu.write_palette_B(address, (word & 0xFFFF) as u16);
                        self.gpu.write_palette_B(address + 2, (word >> 16) as u16);
                    }
                    return;
                } else if address >= VRAM_LCDC_A && address < OAM_START {
                    self.gpu.write_lcdc(address, (word & 0xFFFF) as u16);
                    self.gpu.write_lcdc(address + 2, (word >> 16) as u16);
                    return;
                } else if address >= VRAM_BGA_START && address < VRAM_BGB_START {
                    self.gpu.write_bga(address, (word & 0xFFFF) as u16);
                    self.gpu.write_bga(address + 2, (word >> 16) as u16);
                    return;
                } else if address >= VRAM_BGB_START && address < VRAM_OBJA_START {
                    self.gpu.write_bgb(address, (word & 0xFFFF) as u16);
                    self.gpu.write_bgb(address + 2, (word >> 16) as u16);
                    return;
                } else if address >= VRAM_OBJA_START && address < VRAM_OBJB_START {
                    self.gpu.write_obja(address, (word & 0xFFFF) as u16);
                    self.gpu.write_obja(address + 2, (word >> 16) as u16);
                    return;
                } else if address >= VRAM_OBJB_START && address < VRAM_LCDC_A {
                    self.gpu.write_objb(address, (word & 0xFFFF) as u16);
                    self.gpu.write_objb(address + 2, (word >> 16) as u16);
                    return;
                } else if address >= OAM_START && address < GBA_ROM_START {
                    self.gpu.write_OAM(address, (word & 0xFFFF) as u16);
                    self.gpu.write_OAM(address + 2, (word >> 16) as u16);
                    return;
                } else {
                    println!(
                        "(9) Unrecognized word write of ${:08X} to ${:08X}",
                        word, address
                    );
                }
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
        // Main RAM
        if address >= MAIN_RAM_START && address < SHARED_WRAM_START {
            let idx = (address & MAIN_RAM_MASK) as usize;
            self.main_ram[idx..idx + 2].copy_from_slice(&halfword.to_le_bytes());
            return;
        }

        // Palette memory
        if address >= PALETTE_START && address < VRAM_BGA_START {
            if (address & 0x7FF) < 0x400 {
                self.gpu.write_palette_a(address, halfword);
            } else {
                self.gpu.write_palette_b(address, halfword);
            }
            return;
        }

        // LCDC VRAM
        if address >= VRAM_LCDC_A && address < OAM_START {
            self.gpu.write_lcdc(address, halfword);
            return;
        }

        // Background VRAM
        if address >= VRAM_BGA_START && address < VRAM_BGB_START {
            self.gpu.write_bga(address, halfword);
            return;
        }
        if address >= VRAM_BGB_START && address < VRAM_OBJA_START {
            self.gpu.write_bgb(address, halfword);
            return;
        }

        // Object VRAM
        if address >= VRAM_OBJA_START && address < VRAM_OBJB_START {
            self.gpu.write_obja(address, halfword);
            return;
        }
        if address >= VRAM_OBJB_START && address < VRAM_LCDC_A {
            self.gpu.write_objb(address, halfword);
            return;
        }

        // OAM
        if address >= OAM_START && address < GBA_ROM_START {
            self.gpu.write_oam(address, halfword);
            return;
        }

        // Shared WRAM
        if address >= SHARED_WRAM_START && address < IO_REGS_START {
            match self.wramcnt {
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
                3 => {
                    // Undefined memory, do nothing
                }
                _ => {}
            }
            return;
        }

        // IO registers
        match address {
            0x04000000 => self.gpu.set_disp_cnt_a_lo(halfword),
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
            0x04000054 => self.gpu.set_bldy_a(halfword),
            0x04000060 => self.gpu.set_disp3d_cnt(halfword),
            0x0400006C => self.gpu.set_master_bright_a(halfword),
            0x040000BA => self.dma.write_cnt(0, halfword),
            0x040000C6 => self.dma.write_cnt(1, halfword),
            0x040000D0 => self.dma.write_len(2, halfword),
            0x040000D2 => self.dma.write_cnt(2, halfword),
            0x040000DE => self.dma.write_cnt(3, halfword),
            0x04000100 => self.timers.write_lo(halfword, 4),
            0x04000102 => self.timers.write_hi(halfword, 4),
            0x04000104 => self.timers.write_lo(halfword, 5),
            0x04000106 => self.timers.write_hi(halfword, 5),
            0x04000108 => self.timers.write_lo(halfword, 6),
            0x0400010A => self.timers.write_hi(halfword, 6),
            0x0400010C => self.timers.write_lo(halfword, 7),
            0x0400010E => self.timers.write_hi(halfword, 7),
            0x04000180 => {
                self.ipcsync_nds9.write(halfword);
                self.ipcsync_nds7.receive_input(halfword);
                if (halfword & (1 << 13) != 0) && self.ipcsync_nds7.irq_enable {
                    self.request_interrupt7(Interrupt::IPCSYNC);
                }
            }
            0x04000184 => {
                self.fifo9.write_cnt(halfword);
                if self.fifo9.request_empty_irq {
                    self.request_interrupt9(Interrupt::IPC_FIFO_EMPTY);
                    self.fifo9.request_empty_irq = false;
                }
                if self.fifo9.request_nempty_irq {
                    self.request_interrupt9(Interrupt::IPC_FIFO_NEMPTY);
                    self.fifo9.request_nempty_irq = false;
                }
            }
            0x040001A0 => self.cart.set_auxspicnt(halfword),
            0x04000204 => self.exmemcnt = halfword,
            0x04000208 => self.int9_reg.ime = halfword & 0x1,
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
            0x04000300 => self.postflg9 = halfword & 0x1,
            0x04000304 => self.gpu.set_powcnt1(halfword),
            0x04000340 => {} // TODO: ALPHA_TEST_REF
            0x04000354 => self.gpu.set_clear_depth(halfword),
            0x04000356 => {} // TODO: CLRIMAGE_OFFSET
            0x0400035C => {} // TODO: FOG_OFFSET
            0x04001000 => self.gpu.set_disp_cnt_b_lo(halfword),
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
            0x04001054 => self.gpu.set_bldy_b(halfword),
            0x0400106C => self.gpu.set_master_bright_b(halfword),
            _ => {
                // EDGE_COLOR region (0x04000330 - 0x04000340)
                if address >= 0x04000330 && address < 0x04000340 {
                    return;
                }
                // TOON table
                if address >= 0x04000380 && address < 0x040003C0 {
                    self.gpu
                        .set_toon_table(((address & 0x3F) >> 1) as usize, halfword);
                    return;
                }
                // GBA ROM
                if address >= GBA_ROM_START {
                    return;
                }

                println!(
                    "\n(9) Unrecognized halfword write of ${:04X} to ${:08X}",
                    halfword, address
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
        // Main RAM
        if address >= MAIN_RAM_START && address < SHARED_WRAM_START {
            let idx = (address & MAIN_RAM_MASK) as usize;
            self.main_ram[idx] = byte;
            return;
        }

        // Palette memory (ignored for byte writes)
        if address >= PALETTE_START && address < VRAM_BGA_START {
            return;
        }

        // IO registers / special addresses
        match address {
            0x0400004C => self.gpu.set_mosaic_a(byte),
            0x040001A1 => self.cart.set_hi_auxspicnt(byte),
            0x040001A2 => {
                println!("\nAUXSPIDATA: ${:02X}", byte);
                self.cart.set_auxspidata(byte);
            }
            0x040001A8..=0x040001AF => {
                self.cart
                    .receive_command(byte, (address - 0x040001A8) as usize);
            }
            0x04000208 => self.int9_reg.ime = byte & 0x1,
            0x04000240 => self.gpu.set_vramcnt_a(byte),
            0x04000241 => self.gpu.set_vramcnt_b(byte),
            0x04000242 => self.gpu.set_vramcnt_c(byte),
            0x04000243 => self.gpu.set_vramcnt_d(byte),
            0x04000244 => self.gpu.set_vramcnt_e(byte),
            0x04000245 => self.gpu.set_vramcnt_f(byte),
            0x04000246 => self.gpu.set_vramcnt_g(byte),
            0x04000247 => self.wramcnt = byte & 0x3,
            0x04000248 => self.gpu.set_vramcnt_h(byte),
            0x04000249 => self.gpu.set_vramcnt_i(byte),
            0x0400104C => self.gpu.set_mosaic_b(byte),
            0x04001054 => self.gpu.set_bldy_b(byte),
            _ => {
                // Warn for 8-bit VRAM writes
                if address >= PALETTE_START && address < GBA_ROM_START {
                    println!("\nWarning: 8-bit write to VRAM ${:08X}", address);
                    return;
                }

                // Unrecognized byte write
                println!(
                    "\n(9) Unrecognized byte write of ${:02X} to ${:08X}",
                    byte, address
                );
            }
        }
    }
}
