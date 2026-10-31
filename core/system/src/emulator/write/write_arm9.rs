// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
use ds_gpu::engine_3d::state::Gpu3D;
use ds_mem_addr::*;
use ds_peripherals::ipc::Side;

use super::Emulator;

impl Emulator {
    /// Writes a 32-bit word to ARM9 memory-mapped address space
    ///
    /// ```text
    /// from: write_word(Arm9) / ARM9 DMA (dma_handle_event)
    ///   |
    ///   v
    /// arm9_write_word(address, word)
    ///   1. 3D registers (toon/fog/edge tables...) -> engine_3d.write_reg
    ///   2. 2D engine registers                    -> write_io_2d_u32
    ///   3. by address:
    ///      main RAM / shared WRAM
    ///      DMA, IPC (may raise IRQ), cartridge ROMCTRL/commands, IE/IF
    ///      VRAMCNT, divider -> start_division, sqrt -> start_sqrt
    ///      GXSTAT -> set_gxstat, GXFIFO -> write_gxfifo,
    ///      command ports -> write_fifo_direct
    ///      palette / VRAM / OAM (gpu)
    ///   |
    ///   v
    /// to:   the device; side effects (DMA start, IRQ, 3D command queue)
    ///       happen immediately
    /// ```
    ///
    /// Handles writes to:
    /// - Main RAM
    /// - Shared WRAM (controlled by WRAM CNT)
    /// - GPU registers (BG, WIN, BLDCNT, etc.)
    /// - DMA registers
    /// - IPC, FIFO
    /// - Cartridge AUX SPI registers
    pub fn arm9_write_word(&mut self, address: u32, word: u32) {
        if Gpu3D::is_3d_reg(address) {
            return self.gpu.engine_3d.write_reg(address, word, u32::MAX);
        }
        if crate::emulator::gpu::io_2d::is_2d_reg(address) {
            return self.write_io_2d_u32(address, word);
        }
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
            // DS Video BG Modes / Control: https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol
            0x0400_0000 => self.gpu.set_dispcnt_a(word), // DISPCNT - LCD Control
            // LCD I/O Interrupts and Status: https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus
            0x0400_0004 => self.gpu.set_dispstat9((word & 0xFFFF) as u16), // DISPSTAT - General LCD Status
            // LCD I/O BG Control: https://problemkaputt.de/gbatek.htm#lcdiobgcontrol
            0x0400_0008 => {
                // BG0CNT - BG0 Control (BG Modes 0,1 only)
                self.gpu.set_bgcnt_a((word & 0xFFFF) as u16, 0); // BG0CNT (lower halfword)
                self.gpu.set_bgcnt_a((word >> 16) as u16, 1);
            }
            0x0400_000C => {
                // BG2CNT - BG2 Control (BG Modes 0,1,2 only)
                self.gpu.set_bgcnt_a((word & 0xFFFF) as u16, 2); // BG2CNT (lower halfword)
                self.gpu.set_bgcnt_a((word >> 16) as u16, 3);
            }
            0x0400_000E..=0x0400_0070 => {
                // BG3CNT - BG3 Control (BG Modes 0,2 only) .. TVOUTCNT - Unknown
                // BG offset/affine, window, mosaic, blend, and 3D-select
                // registers. This range was an empty stub, so word stores
                // (common for e.g. `STR Rn, [r0, #0x40]` setting WIN0H/WIN0V
                // together) were silently dropped even though the halfword
                // dispatch below already implements every register here.
                // Decomposing into the two halfwords guarantees parity with
                // it instead of duplicating each register's logic.
                self.arm9_write_halfword(address, (word & 0xFFFF) as u16);
                self.arm9_write_halfword(address + 2, (word >> 16) as u16);
            }
            // DMA0-3 SAD/DAD/CNT (word count in the low halfword, control in
            // the high halfword; `dma_write_len_cnt` splits those for us).
            // Mirrors the ARM7 dispatch in `write_arm7.rs`, which already
            // wires this correctly for its own channels (4-7).
            // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
            0x0400_00B0 => self.dma.write_source(0, word), // DMA0SAD - DMA 0 Source Address
            0x0400_00B4 => self.dma.write_dest(0, word),   // DMA0DAD - DMA 0 Destination Address
            0x0400_00B8 => self.dma_write_len_cnt(0, word), // DMA0CNT_L - DMA 0 Word Count

            0x0400_00BC => self.dma.write_source(1, word), // DMA1SAD - DMA 1 Source Address
            0x0400_00C0 => self.dma.write_dest(1, word),   // DMA1DAD - DMA 1 Destination Address
            0x0400_00C4 => self.dma_write_len_cnt(1, word), // DMA1CNT_L - DMA 1 Word Count

            0x0400_00C8 => self.dma.write_source(2, word), // DMA2SAD - DMA 2 Source Address
            0x0400_00CC => self.dma.write_dest(2, word),   // DMA2DAD - DMA 2 Destination Address
            0x0400_00D0 => self.dma_write_len_cnt(2, word), // DMA2CNT_L - DMA 2 Word Count

            0x0400_00D4 => self.dma.write_source(3, word), // DMA3SAD - DMA 3 Source Address
            0x0400_00D8 => self.dma.write_dest(3, word),   // DMA3DAD - DMA 3 Destination Address
            0x0400_00DC => self.dma_write_len_cnt(3, word), // DMA3CNT_L - DMA 3 Word Count
            0x0400_00E0..=0x0400_00EC => {
                // DMA0FILL - DMA 0 Filldata .. DMA3FILL - DMA 3 Filldata
                let i = ((address - 0x0400_00E0) / 4) as usize; // DMA0FILL - DMA 0 Filldata
                self.dma_fill[i] = word;
            }
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x0400_0180 => {
                // IPCSYNC - IPC Synchronize Register
                let irq = self.ipc.write_sync(Side::Arm9, word as u16, 0xFFFF); // full IPCSYNC halfword
                self.raise_ipc(irq);
            }
            0x0400_0184 => {
                // IPCFIFOCNT - IPC Fifo Control Register
                for irq in self.ipc.write_cnt(Side::Arm9, word as u16) {
                    self.raise_ipc(irq);
                }
            }
            0x0400_0188 => {
                // IPCFIFOSEND - IPC Send Fifo
                #[cfg(feature = "log")]
                log::debug!(target: "ipc", "Arm9 send {word:08X}");
                let irq = self.ipc.send(Side::Arm9, word);
                self.raise_ipc(irq);
            }
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x0400_01A0 => {
                // AUXSPICNT - Gamecard ROM and SPI Control
                self.cart.set_auxspicnt((word & 0xFFFF) as u16); // AUXSPICNT (lower halfword)
                self.cart.set_auxspidata(((word >> 16) & 0xFF) as u8); // AUXSPIDATA (bits 16-23)
            }
            0x0400_01A4 => self.cart.set_romctrl(word), // ROMCTRL - Gamecard Bus ROMCTRL
            0x0410_0010 if !self.arm7_has_cart_rights() => self.cart_write_data(word), // Gamecard bus data out
            0x0400_01A8 => {
                // Gamecard bus 8-byte Command Out
                self.cart.receive_command((word >> 24) as u8, 3);
                self.cart.receive_command(((word >> 16) & 0xFF) as u8, 2); // command byte 2
                self.cart.receive_command(((word >> 8) & 0xFF) as u8, 1); // command byte 1
                self.cart.receive_command((word & 0xFF) as u8, 0); // command byte 0
            }
            0x0400_01AC => {
                // Gamecard bus 8-byte command out (+4h)
                self.cart.receive_command((word >> 24) as u8, 7);
                self.cart.receive_command(((word >> 16) & 0xFF) as u8, 6); // command byte 6
                self.cart.receive_command(((word >> 8) & 0xFF) as u8, 5); // command byte 5
                self.cart.receive_command((word & 0xFF) as u8, 4); // command byte 4
            }
            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x0400_0208 => self.int9_reg.ime = word & 0x1, // IME - Interrupt Master Enable Register
            0x0400_0210 => self.int9_reg.irq_enable = word, // IE - Interrupt Enable
            0x0400_0214 => {
                // IF - Interrupt Request Flags
                self.int9_reg.irq_flags &= !word;
                self.check_gxfifo_irq();
            }
            // VRAMCNT_A..G, WRAMCNT, VRAMCNT_H/I: byte registers that games
            // also write 16/32 bits at a time.
            // DS Memory Control - VRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram
            0x0400_0240 | 0x0400_0244 | 0x0400_0248 => {
                // VRAMCNT_A - VRAM-A (128K) Bank Control / VRAMCNT_E - VRAM-E (64K) Bank Control / VRAMCNT_H - VRAM-H (32K) Bank Control
                for (i, byte) in word.to_le_bytes().into_iter().enumerate() {
                    if address + (i as u32) <= 0x0400_0249 {
                        // VRAMCNT_I - VRAM-I (16K) Bank Control
                        self.arm9_write_byte(address + i as u32, byte);
                    }
                }
            }
            // DS Maths: https://problemkaputt.de/gbatek.htm#dsmaths
            0x0400_0280 => {
                // DIVCNT - Division Control
                self.divcnt = (self.divcnt & 0xC000) | (word as u16 & 0x3); // DIVCNT: keep busy/div-by-zero flags (bits 14-15), set mode (bits 0-1)
                self.start_division();
            }
            0x0400_0290..=0x0400_029C => self.write_div_operand(address, word), // DIV_NUMER - 64bit Division Numerator .. DIV_DENOM - Division Denominator
            0x0400_02B0 => {
                // SQRTCNT - Square Root Control
                self.sqrtcnt = word as u16 & 0x1; // SQRTCNT mode (bit 0)
                self.start_sqrt();
            }
            0x0400_02B8..=0x0400_02BC => self.write_sqrt_param(address, word), // SQRT_PARAM - Square Root Parameter Input
            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            0x0400_0304 => self.gpu.set_powcnt1((word & 0xFFFF) as u16), // POWCNT1 - Graphics Power Control Register
            // DS 3D Status: https://problemkaputt.de/gbatek.htm#ds3dstatus
            0x0400_0600 => self.set_gxstat(word, u32::MAX), // GXSTAT - Geometry Engine Status Register
            // DS Video BG Modes / Control: https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol
            0x0400_1000 => self.gpu.set_dispcnt_b(word), // Engine B DISPCNT - LCD Control
            // DS 3D Geometry Commands: https://problemkaputt.de/gbatek.htm#ds3dgeometrycommands
            0x0400_0400..0x0400_0440 => self.write_gxfifo(word), // GXFIFO - Geometry Command FIFO (40h-byte mirror)
            // DS 3D Matrix Load/Multiply: https://problemkaputt.de/gbatek.htm#ds3dmatrixloadmultiply
            0x0400_0440..0x0400_05CC => self.write_fifo_direct(address, word), // Cmd 10h - MTX_MODE - Set Matrix Mode .. VEC_TEST - Set Directional Vector for Test
            PALETTE_START..VRAM_BGA_START => {
                // LCD Color Palettes: https://problemkaputt.de/gbatek.htm#lcdcolorpalettes
                if (address & 0x7FF) < 0x400 {
                    // first 1K = engine A palette, second 1K = engine B
                    self.gpu.write_palette_a(address, (word & 0xFFFF) as u16); // lower halfword
                    self.gpu.write_palette_a(address + 2, (word >> 16) as u16);
                } else {
                    self.gpu.write_palette_b(address, (word & 0xFFFF) as u16); // lower halfword
                    self.gpu.write_palette_b(address + 2, (word >> 16) as u16);
                }
            }
            VRAM_BGA_START..OAM_START => {
                self.gpu.write_vram_arm9(address & !3, &word.to_le_bytes());
            }

            OAM_START..GBA_ROM_START => {
                self.gpu.write_oam(address, (word & 0xFFFF) as u16); // lower halfword
                self.gpu.write_oam(address + 2, (word >> 16) as u16);
            }
            _ => {
                #[cfg(feature = "log")]
                log::warn!("(9) Unrecognized word write of ${word:08X} to ${address:08X}");
            }
        }
    }

    /// Writes a 16-bit halfword to the specified ARM9 memory address.
    ///
    /// Same routing as [`Self::arm9_write_word`]; 3D registers and GXSTAT
    /// are written with a byte mask, 2D registers through write_io_2d_u16.
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
        let shift = (address & 2) * 8;
        if Gpu3D::is_3d_reg(address) {
            return self.gpu.engine_3d.write_reg(
                address,
                u32::from(halfword) << shift,
                0xFFFF << shift, // halfword lane mask
            );
        }
        // DS 3D Status: https://problemkaputt.de/gbatek.htm#ds3dstatus
        if (0x0400_0600..0x0400_0604).contains(&address) {
            // GXSTAT - Geometry Engine Status Register
            return self.set_gxstat(u32::from(halfword) << shift, 0xFFFF << shift); // halfword lane mask
        }
        if crate::emulator::gpu::io_2d::is_2d_reg(address) {
            return self.write_io_2d_u16(address, halfword);
        }
        // IO registers
        match address {
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let idx = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[idx..idx + 2].copy_from_slice(&halfword.to_le_bytes());
            }

            // Palette memory
            PALETTE_START..VRAM_BGA_START => match address & 0x7FF {
                ..0x400 => self.gpu.write_palette_a(address, halfword), // first 1K = engine A palette
                _ => self.gpu.write_palette_b(address, halfword),
            },

            // LCDC VRAM
            VRAM_BGA_START..OAM_START => {
                self.gpu.write_vram_arm9(address & !1, &halfword.to_le_bytes());
            }

            // OAM
            OAM_START..GBA_ROM_START => {
                self.gpu.write_oam(address, halfword);
            }

            // Shared WRAM
            SHARED_WRAM_START..IO_REGS_START => {
                match self.wram_cnt {
                    0 => {
                        // DS Memory Control - WRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolwram
                        let idx = (address & 0x7FFF) as usize; // WRAMCNT 0: ARM9 gets all 32K
                        self.shared_wram[idx..idx + 2].copy_from_slice(&halfword.to_le_bytes());
                    }
                    1 => {
                        let idx = ((address & 0x3FFF) + 0x4000) as usize; // WRAMCNT 1: ARM9 gets the second 16K
                        self.shared_wram[idx..idx + 2].copy_from_slice(&halfword.to_le_bytes());
                    }
                    2 => {
                        let idx = (address & 0x3FFF) as usize; // WRAMCNT 2: ARM9 gets the first 16K
                        self.shared_wram[idx..idx + 2].copy_from_slice(&halfword.to_le_bytes());
                    }
                    _ => {} // Undefined memory, do nothing
                }
            }
            // DS Video BG Modes / Control: https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol
            0x04000000 => self.gpu.set_dispcnt_a(halfword as u32), // DISPCNT - LCD Control
            // LCD I/O Interrupts and Status: https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus
            0x04000004 => self.gpu.set_dispstat9(halfword), // DISPSTAT - General LCD Status
            // LCD I/O BG Control: https://problemkaputt.de/gbatek.htm#lcdiobgcontrol
            0x04000008 => self.gpu.set_bgcnt_a(halfword, 0), // BG0CNT - BG0 Control (BG Modes 0,1 only)
            0x0400000A => self.gpu.set_bgcnt_a(halfword, 1), // BG1CNT - BG1 Control (BG Modes 0,1 only)
            0x0400000C => self.gpu.set_bgcnt_a(halfword, 2), // BG2CNT - BG2 Control (BG Modes 0,1,2 only)
            0x0400000E => self.gpu.set_bgcnt_a(halfword, 3), // BG3CNT - BG3 Control (BG Modes 0,2 only)
            // LCD I/O BG Scrolling: https://problemkaputt.de/gbatek.htm#lcdiobgscrolling
            0x04000010 => self.gpu.set_bghofs_a(halfword, 0), // BG0HOFS - BG0 X-Offset
            0x04000012 => self.gpu.set_bgvofs_a(halfword, 0), // BG0VOFS - BG0 Y-Offset
            0x04000014 => self.gpu.set_bghofs_a(halfword, 1), // BG1HOFS - BG1 X-Offset
            0x04000016 => self.gpu.set_bgvofs_a(halfword, 1), // BG1VOFS - BG1 Y-Offset
            0x04000018 => self.gpu.set_bghofs_a(halfword, 2), // BG2HOFS - BG2 X-Offset
            0x0400001A => self.gpu.set_bgvofs_a(halfword, 2), // BG2VOFS - BG2 Y-Offset
            0x0400001C => self.gpu.set_bghofs_a(halfword, 3), // BG3HOFS - BG3 X-Offset
            0x0400001E => self.gpu.set_bgvofs_a(halfword, 3), // BG3VOFS - BG3 Y-Offset
            // LCD I/O BG Rotation/Scaling: https://problemkaputt.de/gbatek.htm#lcdiobgrotationscaling
            0x04000020 => self.gpu.set_bg2p_a(halfword, 0), // BG2PA - BG2 Rotation/Scaling Parameter A (alias dx)
            0x04000022 => self.gpu.set_bg2p_a(halfword, 1), // BG2PB - BG2 Rotation/Scaling Parameter B (alias dmx)
            0x04000024 => self.gpu.set_bg2p_a(halfword, 2), // BG2PC - BG2 Rotation/Scaling Parameter C (alias dy)
            0x04000026 => self.gpu.set_bg2p_a(halfword, 3), // BG2PD - BG2 Rotation/Scaling Parameter D (alias dmy)
            0x04000030 => self.gpu.set_bg3p_a(halfword, 0), // BG3PA - BG3 Rotation/Scaling Parameter A (dx)
            0x04000032 => self.gpu.set_bg3p_a(halfword, 1), // BG3PB - BG3 Rotation/Scaling Parameter B (dmx)
            0x04000034 => self.gpu.set_bg3p_a(halfword, 2), // BG3PC - BG3 Rotation/Scaling Parameter C (dy)
            0x04000036 => self.gpu.set_bg3p_a(halfword, 3), // BG3PD - BG3 Rotation/Scaling Parameter D (dmy)
            // LCD I/O Window Feature: https://problemkaputt.de/gbatek.htm#lcdiowindowfeature
            0x04000040 => self.gpu.set_win0h_a(halfword), // WIN0H - Window 0 Horizontal Dimensions
            0x04000042 => self.gpu.set_win1h_a(halfword), // WIN1H - Window 1 Horizontal Dimensions
            0x04000044 => self.gpu.set_win0v_a(halfword), // WIN0V - Window 0 Vertical Dimensions
            0x04000046 => self.gpu.set_win1v_a(halfword), // WIN1V - Window 1 Vertical Dimensions
            0x04000048 => self.gpu.set_winin_a(halfword), // WININ - Control of Inside of Window(s)
            0x0400004A => self.gpu.set_winout_a(halfword), // WINOUT - Control of Outside of Windows & Inside of OBJ Window
            // LCD I/O Mosaic Function: https://problemkaputt.de/gbatek.htm#lcdiomosaicfunction
            0x0400004C => self.gpu.set_mosaic_a(halfword), // MOSAIC - Mosaic Size
            // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
            0x04000050 => {
                // BLDCNT - Color Special Effects Selection
                self.gpu.set_bldcnt_a(halfword);
            }
            0x04000052 => self.gpu.set_bldalpha_a(halfword), // BLDALPHA - Alpha Blending Coefficients
            0x04000054 => {
                // BLDY - Brightness (Fade-In/Out) Coefficient
                self.gpu.set_bldy_a(halfword as u8);
            }
            // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
            0x04000060 => self.gpu.set_disp3dcnt(halfword), // DISP3DCNT - 3D Display Control Register
            // DS Video Stuff: https://problemkaputt.de/gbatek.htm#dsvideostuff
            0x0400006C => {
                // MASTER_BRIGHT - Master Brightness Up/Down
                self.gpu.set_master_bright_a(halfword);
            }
            // DMA0-3 SAD/DAD/CNT halfword sub-writes. Previously only the
            // control halfword (and DMA2's length) were wired here, so
            // source/destination addresses set via halfword stores (STRH)
            // were silently dropped and every transfer read/wrote address 0.
            // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
            0x040000B0 => self.dma.write_source_lo(0, halfword), // DMA0SAD - DMA 0 Source Address
            0x040000B2 => self.dma.write_source_hi(0, halfword), // DMA0SAD - DMA 0 Source Address (upper halfword)
            0x040000B4 => self.dma.write_dest_lo(0, halfword), // DMA0DAD - DMA 0 Destination Address
            0x040000B6 => self.dma.write_dest_hi(0, halfword), // DMA0DAD - DMA 0 Destination Address (upper halfword)
            0x040000B8 => self.dma.write_len(0, halfword),     // DMA0CNT_L - DMA 0 Word Count
            0x04000132 => self.write_keycnt(0, halfword),      // KEYCNT - Key Interrupt Control
            0x040000BA => self.dma_write_cnt(0, halfword),     // DMA0CNT_H - DMA 0 Control

            0x040000BC => self.dma.write_source_lo(1, halfword), // DMA1SAD - DMA 1 Source Address
            0x040000BE => self.dma.write_source_hi(1, halfword), // DMA1SAD - DMA 1 Source Address (upper halfword)
            0x040000C0 => self.dma.write_dest_lo(1, halfword), // DMA1DAD - DMA 1 Destination Address
            0x040000C2 => self.dma.write_dest_hi(1, halfword), // DMA1DAD - DMA 1 Destination Address (upper halfword)
            0x040000C4 => self.dma.write_len(1, halfword),     // DMA1CNT_L - DMA 1 Word Count
            0x040000C6 => self.dma_write_cnt(1, halfword),     // DMA1CNT_H - DMA 1 Control

            0x040000C8 => self.dma.write_source_lo(2, halfword), // DMA2SAD - DMA 2 Source Address
            0x040000CA => self.dma.write_source_hi(2, halfword), // DMA2SAD - DMA 2 Source Address (upper halfword)
            0x040000CC => self.dma.write_dest_lo(2, halfword), // DMA2DAD - DMA 2 Destination Address
            0x040000CE => self.dma.write_dest_hi(2, halfword), // DMA2DAD - DMA 2 Destination Address (upper halfword)
            0x040000D0 => self.dma.write_len(2, halfword),     // DMA2CNT_L - DMA 2 Word Count
            0x040000D2 => self.dma_write_cnt(2, halfword),     // DMA2CNT_H - DMA 2 Control

            0x040000D4 => self.dma.write_source_lo(3, halfword), // DMA3SAD - DMA 3 Source Address
            0x040000D6 => self.dma.write_source_hi(3, halfword), // DMA3SAD - DMA 3 Source Address (upper halfword)
            0x040000D8 => self.dma.write_dest_lo(3, halfword), // DMA3DAD - DMA 3 Destination Address
            0x040000DA => self.dma.write_dest_hi(3, halfword), // DMA3DAD - DMA 3 Destination Address (upper halfword)
            0x040000DC => self.dma.write_len(3, halfword),     // DMA3CNT_L - DMA 3 Word Count
            0x040000DE => self.dma_write_cnt(3, halfword),     // DMA3CNT_H - DMA 3 Control
            // GBA Timers: https://problemkaputt.de/gbatek.htm#gbatimers
            0x04000100 => self.nds_timing.write_lo(halfword, 4), // TM0CNT_L - Timer 0 Counter/Reload
            0x04000102 => self.nds_timing.write_hi(halfword, 4), // TM0CNT_H - Timer 0 Control
            0x04000104 => self.nds_timing.write_lo(halfword, 5), // TM1CNT_L - Timer 1 Counter/Reload
            0x04000106 => self.nds_timing.write_hi(halfword, 5), // TM1CNT_H - Timer 1 Control
            0x04000108 => self.nds_timing.write_lo(halfword, 6), // TM2CNT_L - Timer 2 Counter/Reload
            0x0400010A => self.nds_timing.write_hi(halfword, 6), // TM2CNT_H - Timer 2 Control
            0x0400010C => self.nds_timing.write_lo(halfword, 7), // TM3CNT_L - Timer 3 Counter/Reload
            0x0400010E => self.nds_timing.write_hi(halfword, 7), // TM3CNT_H - Timer 3 Control
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000180 => {
                // IPCSYNC - IPC Synchronize Register
                let irq = self.ipc.write_sync(Side::Arm9, halfword, 0xFFFF); // full IPCSYNC halfword
                self.raise_ipc(irq);
            }
            0x04000184 => {
                // IPCFIFOCNT - IPC Fifo Control Register
                for irq in self.ipc.write_cnt(Side::Arm9, halfword) {
                    self.raise_ipc(irq);
                }
            }
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x040001A0 => self.cart.set_auxspicnt(halfword), // AUXSPICNT - Gamecard ROM and SPI Control
            0x040001A2 => self.cart.set_auxspidata(halfword as u8), // AUXSPIDATA - Gamecard SPI Bus Data/Strobe
            // DS Memory Control - Cartridges and Main RAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolcartridgesandmainram
            0x04000204 => self.ex_mem_cnt = halfword, // EXMEMCNT - External Memory Control
            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x04000208 => self.int9_reg.ime = (halfword & 0x1) as u32, // IME - Interrupt Master Enable Register
            // VRAMCNT_A..I and WRAMCNT are byte registers; 16-bit writes
            // set two of them (Kirby Super Star Ultra writes 0x04000240 and
            // 0x04000242 this way; dropping them left all VRAM unmapped).
            // DS Memory Control - VRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram
            0x04000240 | 0x04000242 | 0x04000244 | 0x04000246 | 0x04000248 => {
                // VRAMCNT_A - VRAM-A (128K) Bank Control / VRAMCNT_C - VRAM-C (128K) Bank Control / VRAMCNT_E - VRAM-E (64K) Bank Control / VRAMCNT_G - VRAM-G (16K) Bank Control / VRAMCNT_H - VRAM-H (32K) Bank Control
                let [lo, hi] = halfword.to_le_bytes();
                self.arm9_write_byte(address, lo);
                self.arm9_write_byte(address + 1, hi);
            }
            // DS Maths: https://problemkaputt.de/gbatek.htm#dsmaths
            0x04000280 => {
                // DIVCNT - Division Control
                self.divcnt = (self.divcnt & 0xC000) | (halfword & 0x3); // DIVCNT: keep busy/div-by-zero flags (bits 14-15), set mode (bits 0-1)
                self.start_division();
            }
            0x04000290..=0x0400029E => {
                // DIV_NUMER - 64bit Division Numerator .. DIV_DENOM - Division Denominator
                // Halfword write into one operand word: merge with the other half.
                let word_addr = address & !3;
                let shift = (address & 2) * 8;
                let current = self.arm9_read_word(word_addr);
                let word = (current & !(0xFFFF << shift)) | (u32::from(halfword) << shift); // replace one halfword of the 32-bit word
                self.write_div_operand(word_addr, word);
            }
            0x040002B0 => {
                // SQRTCNT - Square Root Control
                self.sqrtcnt = halfword & 0x1; // SQRTCNT mode (bit 0)
                self.start_sqrt();
            }
            0x040002B8..=0x040002BE => {
                // SQRT_PARAM - Square Root Parameter Input
                let word_addr = address & !3;
                let shift = (address & 2) * 8;
                let current = self.arm9_read_word(word_addr);
                let word = (current & !(0xFFFF << shift)) | (u32::from(halfword) << shift); // replace one halfword of the 32-bit word
                self.write_sqrt_param(word_addr, word);
            }
            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            0x04000300 => self.postflg9 = (halfword & 0x1) as u8, // POSTFLG - Post Boot Flag
            0x04000304 => self.gpu.set_powcnt1(halfword), // POWCNT1 - Graphics Power Control Register
            // DS Video BG Modes / Control: https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol
            0x04001000 => self.gpu.set_dispcnt_b(halfword as u32), // Engine B DISPCNT - LCD Control
            // LCD I/O BG Control: https://problemkaputt.de/gbatek.htm#lcdiobgcontrol
            0x04001008 => self.gpu.set_bgcnt_b(halfword, 0), // Engine B BG0CNT - BG0 Control (BG Modes 0,1 only)
            0x0400100A => self.gpu.set_bgcnt_b(halfword, 1), // Engine B BG1CNT - BG1 Control (BG Modes 0,1 only)
            0x0400100C => self.gpu.set_bgcnt_b(halfword, 2), // Engine B BG2CNT - BG2 Control (BG Modes 0,1,2 only)
            0x0400100E => self.gpu.set_bgcnt_b(halfword, 3), // Engine B BG3CNT - BG3 Control (BG Modes 0,2 only)
            // LCD I/O BG Scrolling: https://problemkaputt.de/gbatek.htm#lcdiobgscrolling
            0x04001010 => self.gpu.set_bghofs_b(halfword, 0), // Engine B BG0HOFS - BG0 X-Offset
            0x04001012 => self.gpu.set_bgvofs_b(halfword, 0), // Engine B BG0VOFS - BG0 Y-Offset
            0x04001014 => self.gpu.set_bghofs_b(halfword, 1), // Engine B BG1HOFS - BG1 X-Offset
            0x04001016 => self.gpu.set_bgvofs_b(halfword, 1), // Engine B BG1VOFS - BG1 Y-Offset
            0x04001018 => self.gpu.set_bghofs_b(halfword, 2), // Engine B BG2HOFS - BG2 X-Offset
            0x0400101A => self.gpu.set_bgvofs_b(halfword, 2), // Engine B BG2VOFS - BG2 Y-Offset
            0x0400101C => self.gpu.set_bghofs_b(halfword, 3), // Engine B BG3HOFS - BG3 X-Offset
            0x0400101E => self.gpu.set_bgvofs_b(halfword, 3), // Engine B BG3VOFS - BG3 Y-Offset
            // LCD I/O BG Rotation/Scaling: https://problemkaputt.de/gbatek.htm#lcdiobgrotationscaling
            0x04001020 => self.gpu.set_bg2p_b(halfword, 0), // Engine B BG2PA - BG2 Rotation/Scaling Parameter A (alias dx)
            0x04001022 => self.gpu.set_bg2p_b(halfword, 1), // Engine B BG2PB - BG2 Rotation/Scaling Parameter B (alias dmx)
            0x04001024 => self.gpu.set_bg2p_b(halfword, 2), // Engine B BG2PC - BG2 Rotation/Scaling Parameter C (alias dy)
            0x04001026 => self.gpu.set_bg2p_b(halfword, 3), // Engine B BG2PD - BG2 Rotation/Scaling Parameter D (alias dmy)
            0x04001030 => self.gpu.set_bg3p_b(halfword, 0), // Engine B BG3PA - BG3 Rotation/Scaling Parameter A (dx)
            0x04001032 => self.gpu.set_bg3p_b(halfword, 1), // Engine B BG3PB - BG3 Rotation/Scaling Parameter B (dmx)
            0x04001034 => self.gpu.set_bg3p_b(halfword, 2), // Engine B BG3PC - BG3 Rotation/Scaling Parameter C (dy)
            0x04001036 => self.gpu.set_bg3p_b(halfword, 3), // Engine B BG3PD - BG3 Rotation/Scaling Parameter D (dmy)
            // LCD I/O Window Feature: https://problemkaputt.de/gbatek.htm#lcdiowindowfeature
            0x04001040 => self.gpu.set_win0h_b(halfword), // Engine B WIN0H - Window 0 Horizontal Dimensions
            0x04001042 => self.gpu.set_win1h_b(halfword), // Engine B WIN1H - Window 1 Horizontal Dimensions
            0x04001044 => self.gpu.set_win0v_b(halfword), // Engine B WIN0V - Window 0 Vertical Dimensions
            0x04001046 => self.gpu.set_win1v_b(halfword), // Engine B WIN1V - Window 1 Vertical Dimensions
            0x04001048 => self.gpu.set_winin_b(halfword), // Engine B WININ - Control of Inside of Window(s)
            0x0400104A => self.gpu.set_winout_b(halfword), // Engine B WINOUT - Control of Outside of Windows & Inside of OBJ Window
            // LCD I/O Mosaic Function: https://problemkaputt.de/gbatek.htm#lcdiomosaicfunction
            0x0400104C => self.gpu.set_mosaic_b(halfword), // Engine B MOSAIC - Mosaic Size
            // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
            0x04001050 => {
                // Engine B BLDCNT - Color Special Effects Selection
                self.gpu.set_bldcnt_b(halfword);
            }
            0x04001052 => self.gpu.set_bldalpha_b(halfword), // Engine B BLDALPHA - Alpha Blending Coefficients
            0x04001054 => {
                // Engine B BLDY - Brightness (Fade-In/Out) Coefficient
                self.gpu.set_bldy_b(halfword as u8);
            }
            // DS Video Stuff: https://problemkaputt.de/gbatek.htm#dsvideostuff
            0x0400106C => self.gpu.set_master_bright_b(halfword), // Engine B MASTER_BRIGHT - Master Brightness Up/Down
            GBA_ROM_START.. => {}                                 // GBA ROM
            _ => {
                #[cfg(feature = "log")]
                log::warn!(
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
        let shift = (address & 3) * 8;
        if Gpu3D::is_3d_reg(address) {
            return self.gpu.engine_3d.write_reg(address, u32::from(byte) << shift, 0xFF << shift); // byte lane mask
        }
        // DS 3D Status: https://problemkaputt.de/gbatek.htm#ds3dstatus
        if (0x0400_0600..0x0400_0604).contains(&address) {
            // GXSTAT - Geometry Engine Status Register
            return self.set_gxstat(u32::from(byte) << shift, 0xFF << shift); // byte lane mask
        }
        if crate::emulator::gpu::io_2d::is_2d_reg(address) {
            return self.write_io_2d_u8(address, byte);
        }
        // IO registers / special addresses
        match address {
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000181 => {
                // IPCSYNC - IPC Synchronize Register (byte 1)
                let irq = self.ipc.write_sync(Side::Arm9, u16::from(byte) << 8, 0xFF00); // IPCSYNC upper byte (output bits 8-11, IRQ bits 13-14)
                self.raise_ipc(irq);
            }
            0x04000180 => {} // IPCSYNC low byte is read-only
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let idx = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[idx] = byte;
            }
            // LCD I/O Mosaic Function: https://problemkaputt.de/gbatek.htm#lcdiomosaicfunction
            0x0400004C => self.gpu.set_mosaic_a(byte as u16), // MOSAIC - Mosaic Size
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x040001A0 => self.cart.set_lo_auxspicnt(byte), // AUXSPICNT - Gamecard ROM and SPI Control
            0x040001A1 => self.cart.set_hi_auxspicnt(byte), // AUXSPICNT - Gamecard ROM and SPI Control (byte 1)
            0x040001A2 => {
                // AUXSPIDATA - Gamecard SPI Bus Data/Strobe
                // #[cfg(feature = "log")]
                // log::trace!("AUXSPIDATA: {byte:02X}");
                self.cart.set_auxspidata(byte);
            }
            0x040001A8..=0x040001AF => {
                // Gamecard bus 8-byte Command Out
                self.cart.receive_command(byte, (address - 0x040001A8) as usize); // Gamecard bus 8-byte Command Out
            }
            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x04000208 => self.int9_reg.ime = (byte & 0x1) as u32, // IME - Interrupt Master Enable Register
            // DS Memory Control - VRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram
            0x04000240 => self.gpu.set_vramcnt_a(byte), // VRAMCNT_A - VRAM-A (128K) Bank Control
            0x04000241 => self.gpu.set_vramcnt_b(byte), // VRAMCNT_B - VRAM-B (128K) Bank Control
            0x04000242 => self.gpu.set_vramcnt_c(byte), // VRAMCNT_C - VRAM-C (128K) Bank Control
            0x04000243 => self.gpu.set_vramcnt_d(byte), // VRAMCNT_D - VRAM-D (128K) Bank Control
            0x04000244 => self.gpu.set_vramcnt_e(byte), // VRAMCNT_E - VRAM-E (64K) Bank Control
            0x04000245 => self.gpu.set_vramcnt_f(byte), // VRAMCNT_F - VRAM-F (16K) Bank Control
            0x04000246 => self.gpu.set_vramcnt_g(byte), // VRAMCNT_G - VRAM-G (16K) Bank Control
            // DS Memory Control - WRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolwram
            0x04000247 => self.wram_cnt = byte & 0x3, // WRAMCNT - WRAM Bank Control
            // DS Memory Control - VRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram
            0x04000248 => self.gpu.set_vramcnt_h(byte), // VRAMCNT_H - VRAM-H (32K) Bank Control
            0x04000249 => self.gpu.set_vramcnt_i(byte), // VRAMCNT_I - VRAM-I (16K) Bank Control
            // LCD I/O Mosaic Function: https://problemkaputt.de/gbatek.htm#lcdiomosaicfunction
            0x0400104C => self.gpu.set_mosaic_b(byte as u16), // Engine B MOSAIC - Mosaic Size
            // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
            0x04001054 => self.gpu.set_bldy_b(byte), // Engine B BLDY - Brightness (Fade-In/Out) Coefficient

            // 0x0500_0000..0x0600_0000
            PALETTE_START..VRAM_BGA_START => {
                #[cfg(feature = "log")]
                log::warn!("\nPalette memory (ignored for byte writes) ${address:08X}");
            }
            // 0x0500_0000..0x0800_0000
            // PALETTE_START..GBA_ROM_START => {
            // 8-bit writes reach VRAM on the DS; OAM ignores them (GBATEK).
            VRAM_BGA_START..OAM_START => self.gpu.write_vram_arm9(address, &[byte]),
            OAM_START..GBA_ROM_START => {}
            _ => {
                // Unrecognized byte write
                #[cfg(feature = "log")]
                log::warn!("\n(9) Unrecognized byte write of ${byte:02X} to ${address:08X}");
            }
        }
    }
}
