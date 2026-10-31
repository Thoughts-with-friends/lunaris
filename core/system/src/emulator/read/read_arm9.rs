// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
use ds_gpu::engine_3d::state::Gpu3D;
use ds_mem_addr::*;
use ds_peripherals::ipc::Side;

use super::Emulator;
impl Emulator {
    /// ARM9 32-bit read: the ARM9 memory map (TCM is handled earlier by
    /// read_word()).
    ///
    /// ```text
    /// from: read_word(Arm9) / ARM9 DMA (dma_handle_event)
    ///   |
    ///   v
    /// arm9_read_word(address)
    ///   0xFFFF_0000..             ARM9 BIOS
    ///   0x02xx_xxxx               main RAM
    ///   0x03xx_xxxx               shared WRAM (WRAMCNT)
    ///   0x0400_0000..             2D engine A/B, DMA, IPC, cartridge,
    ///                             IE/IF/IME, VRAMCNT, divider, sqrt,
    ///                             GXSTAT and 3D registers
    ///   0x0410_0000               IPC FIFO receive (may raise IRQ)
    ///   0x0410_0010               cartridge data -> cart_read_data()
    ///   0x05/0x06/0x07xx_xxxx     palette / VRAM / OAM (gpu)
    ///   anything else             0 (logged with the `log` feature)
    /// ```
    /// # Panics
    #[expect(clippy::unwrap_used)]
    pub fn arm9_read_word(&mut self, address: u32) -> u32 {
        if Gpu3D::is_3d_reg(address) {
            return self.gpu.engine_3d.read_reg(address);
        }
        if crate::emulator::gpu::io_2d::is_2d_reg(address) {
            return self.read_io_2d_u32(address);
        }
        match address {
            // ARM9 BIOS
            0xFFFF_0000..=0xFFFF_FFFF => {
                // DS Memory Maps: https://problemkaputt.de/gbatek.htm#dsmemorymaps
                let offset = (address - 0xFFFF_0000) as usize; // ARM9 BIOS at FFFF0000h
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
            // https://problemkaputt.de/gbatek.htm#dsiomaps
            // DS Video BG Modes / Control: https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol
            0x04000000 => self.gpu.get_dispcnt_a(), // DISPCNT - LCD Control
            // LCD I/O BG Scrolling: https://problemkaputt.de/gbatek.htm#lcdiobgscrolling
            0x04000010 => {
                // BG0HOFS - BG0 X-Offset
                self.gpu.get_bghofs_a(0) as u32 | ((self.gpu.get_bgvofs_a(0) as u32) << 16)
            }
            0x04000014 => {
                // BG1HOFS - BG1 X-Offset
                self.gpu.get_bghofs_a(1) as u32 | ((self.gpu.get_bgvofs_a(1) as u32) << 16)
            }
            0x04000018 => {
                // BG2HOFS - BG2 X-Offset
                self.gpu.get_bghofs_a(2) as u32 | ((self.gpu.get_bgvofs_a(2) as u32) << 16)
            }
            0x0400001C => {
                // BG3HOFS - BG3 X-Offset
                self.gpu.get_bghofs_a(3) as u32 | ((self.gpu.get_bgvofs_a(3) as u32) << 16)
            }
            // DS Video Capture and Main Memory Display Mode: https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode
            0x04000064 => self.gpu.get_dispcapcnt_a(), // DISPCAPCNT - Display Capture Control Register
            // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
            0x040000B0 => self.dma.read_source(0), // DMA0SAD - DMA 0 Source Address
            0x040000B8 => self.dma.read_len(0) as u32 | ((self.dma.read_cnt(0) as u32) << 16), // DMA0CNT_L - DMA 0 Word Count
            0x040000C4 => self.dma.read_len(1) as u32 | ((self.dma.read_cnt(1) as u32) << 16), // DMA1CNT_L - DMA 1 Word Count
            0x040000D0 => self.dma.read_len(2) as u32 | ((self.dma.read_cnt(2) as u32) << 16), // DMA2CNT_L - DMA 2 Word Count
            0x040000DC => self.dma.read_len(3) as u32 | ((self.dma.read_cnt(3) as u32) << 16), // DMA3CNT_L - DMA 3 Word Count
            0x040000E0 => self.dma_fill[0], // DMA0FILL - DMA 0 Filldata
            0x040000E4 => self.dma_fill[1], // DMA1FILL - DMA 1 Filldata
            0x040000E8 => self.dma_fill[2], // DMA2FILL - DMA 2 Filldata
            0x040000EC => self.dma_fill[3], // DMA3FILL - DMA 3 Filldata
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000180 => self.ipc.read_sync(Side::Arm9).into(), // IPCSYNC - IPC Synchronize Register
            0x04000184 => self.ipc.read_cnt(Side::Arm9).into(), // IPCFIFOCNT - IPC Fifo Control Register
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x040001A0 => {
                // AUXSPICNT - Gamecard ROM and SPI Control
                (u32::from(self.cart.get_auxspicnt()))
                    | (u32::from(self.cart.read_auxspidata()) << 16) // AUXSPIDATA in the upper halfword
            }
            0x040001A4 => self.cart.get_romctrl(), // ROMCTRL - Gamecard Bus ROMCTRL
            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x04000208 => self.int9_reg.ime, // IME - Interrupt Master Enable Register
            0x04000210 => self.int9_reg.irq_enable, // IE - Interrupt Enable
            0x04000214 => self.int9_reg.irq_flags, // IF - Interrupt Request Flags
            // DS Memory Control - VRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram
            0x04000240 => self.gpu.get_vramcnt_a() as u32, // VRAMCNT_A - VRAM-A (128K) Bank Control
            // DS Maths: https://problemkaputt.de/gbatek.htm#dsmaths
            0x04000290 => (self.div_numer & 0xFFFF_FFFF) as u32, // DIV_NUMER - 64bit Division Numerator
            0x04000294 => (self.div_numer >> 32) as u32, // DIV_NUMER - Division Numerator (+4h)
            0x04000298 => (self.div_denom & 0xFFFF_FFFF) as u32, // DIV_DENOM - 64bit Division Denominator
            0x0400029C => (self.div_denom >> 32) as u32, // DIV_DENOM - Division Denominator (+4h)
            0x040002A0 => (self.div_result & 0xFFFF_FFFF) as u32, // DIV_RESULT - 64bit Division Quotient (=Numer/Denom)
            0x040002A4 => (self.div_result >> 32) as u32, // DIV_RESULT - Division Quotient (=Numer/Denom) (+4h)
            0x040002A8 => (self.div_remresult & 0xFFFF_FFFF) as u32, // DIVREM_RESULT - 64bit Remainder (=Numer MOD Denom)
            0x040002AC => (self.div_remresult >> 32) as u32, // DIVREM_RESULT - Division Remainder (=Numer MOD Denom) (+4h)
            0x04000280 => u32::from(self.divcnt),            // DIVCNT - Division Control
            0x040002B0 => u32::from(self.sqrtcnt),           // SQRTCNT - Square Root Control
            0x040002B4 => self.sqrt_result,                  // SQRT_RESULT - Square Root Result
            0x040002B8 => (self.sqrt_param & 0xFFFF_FFFF) as u32, // SQRT_PARAM - Square Root Parameter Input
            0x040002BC => (self.sqrt_param >> 32) as u32, // SQRT_PARAM - Square Root Parameter Input (+4h)
            // DS 3D Status: https://problemkaputt.de/gbatek.htm#ds3dstatus
            0x04000600 => self.gpu.engine_3d.get_gxstat(), // GXSTAT - Geometry Engine Status Register
            // DS Video BG Modes / Control: https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol
            0x04001000 => self.gpu.get_dispcnt_b(), // Engine B DISPCNT - LCD Control
            // LCD I/O BG Scrolling: https://problemkaputt.de/gbatek.htm#lcdiobgscrolling
            0x04001010 => {
                // Engine B BG0HOFS - BG0 X-Offset
                self.gpu.get_bghofs_b(0) as u32 | ((self.gpu.get_bgvofs_b(0) as u32) << 16)
            }
            0x04001014 => {
                // Engine B BG1HOFS - BG1 X-Offset
                self.gpu.get_bghofs_b(1) as u32 | ((self.gpu.get_bgvofs_b(1) as u32) << 16)
            }
            0x04001018 => {
                // Engine B BG2HOFS - BG2 X-Offset
                self.gpu.get_bghofs_b(2) as u32 | ((self.gpu.get_bgvofs_b(2) as u32) << 16)
            }
            0x0400101c => {
                // Engine B BG3HOFS - BG3 X-Offset
                self.gpu.get_bghofs_b(3) as u32 | ((self.gpu.get_bgvofs_b(3) as u32) << 16)
            }
            0x04004000 | 0x04004008 => 0,
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04100000 => {
                // IPCFIFORECV - IPC Receive Fifo
                let (word, irq) = self.ipc.receive(Side::Arm9);
                #[cfg(feature = "log")]
                log::debug!(target: "ipc", "Arm9 recv {word:08X}");
                self.raise_ipc(irq);
                word
            }
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x04100010 => self.cart_read_data(), // Gamecard bus 4-byte Data In
            PALETTE_START..VRAM_BGA_START => {
                // LCD Color Palettes: https://problemkaputt.de/gbatek.htm#lcdcolorpalettes
                if (address & 0x7FF) < 0x400 {
                    // first 1K = engine A palette, second 1K = engine B
                    let lo = self.gpu.read_palette_a(address) as u32;
                    let hi = self.gpu.read_palette_a(address + 2) as u32;
                    lo | (hi << 16) // upper halfword
                } else {
                    let lo = self.gpu.read_palette_b(address) as u32;
                    let hi = self.gpu.read_palette_b(address + 2) as u32;
                    lo | (hi << 16) // upper halfword
                }
            }

            VRAM_BGA_START..OAM_START => u32::from_le_bytes(self.gpu.read_vram_arm9(address & !3)),
            OAM_START..GBA_ROM_START => self.gpu.read_oam_u32(address),

            _ => {
                if address >= GBA_ROM_START {
                    0xFFFF_FFFF // open bus
                } else {
                    #[cfg(feature = "log")]
                    log::warn!("ARM9: Unrecognized word read from ${address:08X}");
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
    ///
    /// # Panics
    #[expect(clippy::unwrap_used)]
    pub fn arm9_read_halfword(&self, address: u32) -> u16 {
        if Gpu3D::is_3d_reg(address) {
            return (self.gpu.engine_3d.read_reg(address) >> ((address & 2) * 8)) as u16;
        }
        if crate::emulator::gpu::io_2d::is_2d_reg(address) {
            return self.read_io_2d_u16(address);
        }
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
            VRAM_BGA_START..OAM_START => u16::from_le_bytes(self.gpu.read_vram_arm9(address & !1)),
            OAM_START..GBA_ROM_START => self.gpu.read_oam_u16(address & !1),
            // DS Video BG Modes / Control: https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol
            0x0400_0000 => self.gpu.get_dispcnt_a() as u16, // DISPCNT - LCD Control
            // LCD I/O Interrupts and Status: https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus
            0x0400_0004 => self.gpu.get_dispstat9(), // DISPSTAT - General LCD Status
            0x0400_0006 => self.gpu.get_vcount(),    // VCOUNT - Vertical Counter
            // LCD I/O BG Control: https://problemkaputt.de/gbatek.htm#lcdiobgcontrol
            0x0400_0008 => self.gpu.get_bgcnt_a(0), // BG0CNT - BG0 Control (BG Modes 0,1 only)
            0x0400_000a => self.gpu.get_bgcnt_a(1), // BG1CNT - BG1 Control (BG Modes 0,1 only)
            0x0400_000c => self.gpu.get_bgcnt_a(2), // BG2CNT - BG2 Control (BG Modes 0,1,2 only)
            0x0400_000e => self.gpu.get_bgcnt_a(3), // BG3CNT - BG3 Control (BG Modes 0,2 only)
            // LCD I/O Window Feature: https://problemkaputt.de/gbatek.htm#lcdiowindowfeature
            0x0400_0044 => self.gpu.get_win0v_a(), // WIN0V - Window 0 Vertical Dimensions
            0x0400_0046 => self.gpu.get_win1v_a(), // WIN1V - Window 1 Vertical Dimensions
            0x0400_0048 => self.gpu.get_winin_a(), // WININ - Control of Inside of Window(s)
            0x0400_004a => self.gpu.get_winout_a(), // WINOUT - Control of Outside of Windows & Inside of OBJ Window
            // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
            0x0400_0050 => self.gpu.get_bldcnt_a(), // BLDCNT - Color Special Effects Selection
            0x0400_0052 => self.gpu.get_bldalpha_a(), // BLDALPHA - Alpha Blending Coefficients
            // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
            0x0400_0060 => self.gpu.get_disp3dcnt(), // DISP3DCNT - 3D Display Control Register
            // DS Video Stuff: https://problemkaputt.de/gbatek.htm#dsvideostuff
            0x0400_006c => self.gpu.get_master_bright_a(), // MASTER_BRIGHT - Master Brightness Up/Down
            // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
            0x0400_00ba => self.dma.read_cnt(0), // DMA0CNT_H - DMA 0 Control
            0x0400_00c6 => self.dma.read_cnt(1), // DMA1CNT_H - DMA 1 Control
            0x0400_00d2 => self.dma.read_cnt(2), // DMA2CNT_H - DMA 2 Control
            0x0400_00de => self.dma.read_cnt(3), // DMA3CNT_H - DMA 3 Control
            0x0400_00E0 => (self.dma_fill[0] & 0xFFFF) as u16, // DMA0FILL - DMA 0 Filldata
            0x0400_00E4 => (self.dma_fill[1] & 0xFFFF) as u16, // DMA1FILL - DMA 1 Filldata
            0x0400_00E8 => (self.dma_fill[2] & 0xFFFF) as u16, // DMA2FILL - DMA 2 Filldata
            0x0400_00EC => (self.dma_fill[3] & 0xFFFF) as u16, // DMA3FILL - DMA 3 Filldata
            // GBA Timers: https://problemkaputt.de/gbatek.htm#gbatimers
            0x0400_0100 => self.nds_timing.read_lo(4), // TM0CNT_L - Timer 0 Counter/Reload
            0x0400_0104 => self.nds_timing.read_lo(5), // TM1CNT_L - Timer 1 Counter/Reload
            0x0400_0108 => self.nds_timing.read_lo(6), // TM2CNT_L - Timer 2 Counter/Reload
            0x0400_010C => self.nds_timing.read_lo(7), // TM3CNT_L - Timer 3 Counter/Reload
            // GBA Keypad Input: https://problemkaputt.de/gbatek.htm#gbakeypadinput
            0x0400_0130 => self.key_input.get_value(), // KEYINPUT - Key Status
            0x0400_0132 => self.key_cnt[0],            // KEYCNT - Key Interrupt Control
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x0400_0180 => self.ipc.read_sync(Side::Arm9), // IPCSYNC - IPC Synchronize Register
            0x0400_0184 => self.ipc.read_cnt(Side::Arm9),  // IPCFIFOCNT - IPC Fifo Control Register
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x0400_01A0 => self.cart.get_auxspicnt(), // AUXSPICNT - Gamecard ROM and SPI Control
            0x0400_01A2 => self.cart.read_auxspidata().into(), // AUXSPIDATA - Gamecard SPI Bus Data/Strobe
            // DS Memory Control - Cartridges and Main RAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolcartridgesandmainram
            0x0400_0204 => self.ex_mem_cnt, // EXMEMCNT - External Memory Control
            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x0400_0208 => self.int9_reg.ime as u16, // IME - Interrupt Master Enable Register
            // DS Maths: https://problemkaputt.de/gbatek.htm#dsmaths
            0x0400_0280 => self.divcnt,  // DIVCNT - Division Control
            0x0400_02B0 => self.sqrtcnt, // SQRTCNT - Square Root Control
            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            0x0400_0300 => self.postflg9.into(), // POSTFLG - Post Boot Flag
            0x0400_0304 => self.gpu.get_powcnt1(), // POWCNT1 - Graphics Power Control Register
            // DS 3D Status: https://problemkaputt.de/gbatek.htm#ds3dstatus
            0x0400_0600 => self.gpu.engine_3d.get_gxstat() as u16, // GXSTAT - Geometry Engine Status Register
            // DS 3D I/O Map: https://problemkaputt.de/gbatek.htm#ds3diomap
            0x0400_0602 => (self.gpu.engine_3d.get_gxstat() >> 16) as u16, // GXSTAT - Geometry Engine Status Register (byte 2)
            // DS Video BG Modes / Control: https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol
            0x0400_1000 => (self.gpu.get_dispcnt_b() & 0xffff) as u16, // Engine B DISPCNT - LCD Control
            // LCD I/O BG Control: https://problemkaputt.de/gbatek.htm#lcdiobgcontrol
            0x0400_1008 => self.gpu.get_bgcnt_b(0), // Engine B BG0CNT - BG0 Control (BG Modes 0,1 only)
            0x0400_100a => self.gpu.get_bgcnt_b(1), // Engine B BG1CNT - BG1 Control (BG Modes 0,1 only)
            0x0400_100c => self.gpu.get_bgcnt_b(2), // Engine B BG2CNT - BG2 Control (BG Modes 0,1,2 only)
            0x0400_100e => self.gpu.get_bgcnt_b(3), // Engine B BG3CNT - BG3 Control (BG Modes 0,2 only)
            // LCD I/O Window Feature: https://problemkaputt.de/gbatek.htm#lcdiowindowfeature
            0x0400_1044 => self.gpu.get_win0v_b(), // Engine B WIN0V - Window 0 Vertical Dimensions
            0x0400_1046 => self.gpu.get_win1v_b(), // Engine B WIN1V - Window 1 Vertical Dimensions
            0x0400_1048 => self.gpu.get_winin_b(), // Engine B WININ - Control of Inside of Window(s)
            0x0400_104a => self.gpu.get_winout_b(), // Engine B WINOUT - Control of Outside of Windows & Inside of OBJ Window
            // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
            0x0400_1050 => self.gpu.get_bldcnt_b(), // Engine B BLDCNT - Color Special Effects Selection
            0x0400_1052 => self.gpu.get_bldalpha_b(), // Engine B BLDALPHA - Alpha Blending Coefficients
            // DS Video Stuff: https://problemkaputt.de/gbatek.htm#dsvideostuff
            0x0400_106c => self.gpu.get_master_bright_b(), // Engine B MASTER_BRIGHT - Master Brightness Up/Down
            _ => {
                if address >= GBA_ROM_START {
                    0xFFFF // open bus
                } else {
                    #[cfg(feature = "log")]
                    log::warn!("(9) Unrecognized halfword read from ${:08X}", address);
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
        if Gpu3D::is_3d_reg(address) {
            return (self.gpu.engine_3d.read_reg(address) >> ((address & 3) * 8)) as u8;
        }
        if crate::emulator::gpu::io_2d::is_2d_reg(address) {
            return self.read_io_2d_u8(address);
        }
        // I/O registers
        match address {
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000180 => self.ipc.read_sync(Side::Arm9) as u8, // IPCSYNC - IPC Synchronize Register
            0x04000181 => (self.ipc.read_sync(Side::Arm9) >> 8) as u8, // IPCSYNC - IPC Synchronize Register (byte 1)
            // DS 3D Status: https://problemkaputt.de/gbatek.htm#ds3dstatus
            0x04000600..=0x04000603 => {
                // GXSTAT - Geometry Engine Status Register
                (self.gpu.engine_3d.get_gxstat() >> ((address & 3) * 8)) as u8
            }
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000184 => self.ipc.read_cnt(Side::Arm9) as u8, // IPCFIFOCNT - IPC Fifo Control Register
            0x04000185 => (self.ipc.read_cnt(Side::Arm9) >> 8) as u8, // IPCFIFOCNT - IPC Fifo Control Register (byte 1)
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
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x0400_01a0 => self.cart.get_auxspicnt() as u8, // AUXSPICNT - Gamecard ROM and SPI Control
            0x0400_01a1 => (self.cart.get_auxspicnt() >> 8) as u8, // AUXSPICNT - Gamecard ROM and SPI Control (byte 1)
            0x0400_01a2 => self.cart.read_auxspidata(), // AUXSPIDATA - Gamecard SPI Bus Data/Strobe
            0x0400_01a8..=0x0400_01af => self.cart.read_command((address & 0x7) as usize), // Gamecard bus 8-byte Command Out
            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x0400_0208 => self.int9_reg.ime as u8, // IME - Interrupt Master Enable Register
            // DS Memory Control - VRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram
            0x0400_0240 => self.gpu.get_vramcnt_a(), // VRAMCNT_A - VRAM-A (128K) Bank Control
            0x0400_0241 => self.gpu.get_vramcnt_b(), // VRAMCNT_B - VRAM-B (128K) Bank Control
            // DS Memory Control - WRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolwram
            0x0400_0247 => self.wram_cnt & 0x3, // WRAMCNT - WRAM Bank Control
            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            0x0400_0300 => self.postflg9, // POSTFLG - Post Boot Flag
            0x0400_4000 => 0,             // DSi SCFG registers (04004000h): 0 in DS mode
            PALETTE_START..VRAM_BGA_START => {
                // LCD Color Palettes: https://problemkaputt.de/gbatek.htm#lcdcolorpalettes
                if (address & 0x7FF) < 0x400 {
                    // first 1K = engine A palette, second 1K = engine B
                    (self.gpu.read_palette_a(address) & 0xFF) as u8 // low byte
                } else {
                    (self.gpu.read_palette_b(address) & 0xFF) as u8 // low byte
                }
            }
            VRAM_BGA_START..OAM_START => self.gpu.read_vram_arm9::<1>(address)[0],
            OAM_START..GBA_ROM_START => self.gpu.read_oam_u8(address), // OAM
            GBA_ROM_START.. => 0xFF,                                   // GBA ROM
            _ => {
                // Palette memory
                #[cfg(feature = "log")]
                log::warn!("(9) Unrecognized byte read from ${:08X}", address);
                0
            }
        }
    }
}
