// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later

use ds_mem_addr::*;
use ds_peripherals::ipc::Side;

use super::Emulator;

impl Emulator {
    /// ARM7 32-bit read: the ARM7 memory map.
    ///
    /// ```text
    /// from: read_word(Arm7) / ARM7 DMA (dma_handle_event)
    ///   |
    ///   v
    /// arm7_read_word(address)
    ///   0x0400_0400..0x0400_051F  SPU registers         -> spu.read32
    ///   0x02xx_xxxx               main RAM (4 MB mirrored)
    ///   0x03xx_xxxx               shared WRAM (WRAMCNT) / ARM7 WRAM
    ///   0x0400_01xx               IPC, cartridge, SPI, IE/IF/IME
    ///   0x0410_0000               IPC FIFO receive (may raise IRQ)
    ///   0x0410_0010               cartridge data    -> cart_read_data()
    ///   0x0480_0000..0x0480_FFFF  Wi-Fi ports / RAM -> wifi.read
    ///   0x0000_0000..0x0000_3FFF  ARM7 BIOS (protected outside it)
    ///   0x06xx_xxxx               VRAM banks C/D mapped to the ARM7
    ///   anything else             0 (logged with the `log` feature)
    /// ```
    ///
    /// FIXME: return Result?
    ///
    /// # Panics
    #[expect(clippy::unwrap_used)]
    pub fn arm7_read_word(&mut self, address: u32) -> u32 {
        // DS Sound Channels 0..15: https://problemkaputt.de/gbatek.htm#dssoundchannels015
        if (0x0400_0400..0x0400_0520).contains(&address) {
            // SOUND0CNT - Sound Channel 0 Control .. SNDCAP1LEN - Sound Capture 1 Length
            return self.spu.read32(address);
        }
        match address {
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let off = (address & MAIN_RAM_MASK) as usize;
                u32::from_le_bytes(self.main_ram[off..off + 4].try_into().unwrap())
            }
            // Shared WRAM (per WRAMCNT) and ARM7 WRAM
            SHARED_WRAM_START..IO_REGS_START => {
                let (private, off) = self.arm7_wram_offset(address);
                let mem = if private { &self.arm7_wram } else { &self.shared_wram };
                u32::from_le_bytes([mem[off], mem[off + 1], mem[off + 2], mem[off + 3]])
            }

            // DS I/O Maps: https://problemkaputt.de/gbatek.htm#dsiomaps
            0x04000120 | 0x04000400..0x04000500 => 0, // Debug SIODATA32 / SOUND0CNT - Sound Channel 0 Control .. SOUND15LEN - Sound Channel 15 Length
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000180 => self.ipc.read_sync(Side::Arm7).into(), // IPCSYNC - IPC Synchronize Register
            0x04000184 => self.ipc.read_cnt(Side::Arm7).into(), // IPCFIFOCNT - IPC Fifo Control Register
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x040001A0 => {
                // AUXSPICNT - Gamecard ROM and SPI Control
                (u32::from(self.cart.get_auxspicnt()))
                    | (u32::from(self.cart.read_auxspidata()) << 16) // AUXSPIDATA in the upper halfword
            }
            0x040001A4 => self.cart.get_romctrl(), // ROMCTRL - Gamecard Bus ROMCTRL
            // DS Serial Peripheral Interface Bus (SPI): https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi
            0x040001C0 => (self.spi.get_spicnt() as u32) | (self.spi.read_spidata() as u32) << 16, // SPICNT - SPI Bus Control/Status Register
            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x04000208 => self.int7_reg.ime, // IME - Interrupt Master Enable Register
            0x04000210 => self.int7_reg.irq_enable, // IE - Interrupt Enable
            0x04000214 => self.int7_reg.irq_flags, // IF - Interrupt Request Flags
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04100000 => {
                // IPCFIFORECV - IPC Receive Fifo
                let (word, irq) = self.ipc.receive(Side::Arm7);
                #[cfg(feature = "log")]
                log::debug!(target: "ipc", "Arm7 recv {word:08X}");
                self.raise_ipc(irq);
                word
            }
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x04100010 => self.cart_read_data(), // Gamecard bus 4-byte Data In
            // DS Wifi I/O Map: https://problemkaputt.de/gbatek.htm#dswifiiomap
            0x04800000..0x04810000 => {
                // Wifi WS0/WS1 region (Wifi ports + 8K Wifi RAM)
                u32::from(self.wifi.read(address)) | (u32::from(self.wifi.read(address + 2)) << 16)
            }

            // DS Memory Control - BIOS: https://problemkaputt.de/gbatek.htm#dsmemorycontrolbios
            ..0x4000 => {
                // 16K ARM7 BIOS
                let arm7_pc = self.arm7.get_pc();
                if arm7_pc > 0x4000 || (address < self.bios_prot && arm7_pc > self.bios_prot) {
                    // BIOS is readable only while the PC is inside it (or below BIOSPROT)
                    return 0xFFFF_FFFF; // protected BIOS reads
                }

                let start = address as usize;
                let end = (address + 4) as usize;
                u32::from_le_bytes(self.arm7_bios[start..end].try_into().unwrap())
            }
            0x06000000..0x07000000 => self.gpu.read_arm7_u32(address), // VRAM banks C/D mapped to ARM7
            GBA_ROM_START.. => 0xFFFF_FFFF,                            // GBA slot: open bus

            _ => {
                #[cfg(feature = "log")]
                log::warn!("ARM7: Unrecognized word read from ${address:08X}");
                0
            }
        }
    }

    /// ARM7 16-bit read (same map as [`Self::arm7_read_word`]; adds
    /// DISPSTAT/VCOUNT, DMA, timers, KEYINPUT/EXTKEYIN, RTC, SPI data,
    /// POWCNT2).
    /// # Panics
    #[expect(clippy::unwrap_used, clippy::match_same_arms)]
    pub fn arm7_read_halfword(&mut self, address: u32) -> u16 {
        // DS Sound Channels 0..15: https://problemkaputt.de/gbatek.htm#dssoundchannels015
        if (0x0400_0400..0x0400_0520).contains(&address) {
            // SOUND0CNT - Sound Channel 0 Control .. SNDCAP1LEN - Sound Capture 1 Length
            return self.spu.read16(address);
        }
        match address {
            // BIOS (0x00000000..0x00003FFF)
            ..0x4000 => {
                let pc = self.arm7.get_pc();
                if pc > 0x4000 {
                    // BIOS is readable only while the PC is inside it
                    return 0xFFFF; // protected BIOS reads
                }
                if address < self.bios_prot && pc > self.bios_prot {
                    return 0xFFFF; // protected BIOS reads
                }

                let start = address as usize;
                let end = start + 2;
                u16::from_le_bytes(self.arm7_bios[start..end].try_into().unwrap())
            }

            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let off = (address & MAIN_RAM_MASK) as usize;
                u16::from_le_bytes(self.main_ram[off..off + 2].try_into().unwrap())
            }

            // Shared WRAM (per WRAMCNT) and ARM7 WRAM
            SHARED_WRAM_START..IO_REGS_START => {
                let (private, off) = self.arm7_wram_offset(address);
                let mem = if private { &self.arm7_wram } else { &self.shared_wram };
                u16::from_le_bytes([mem[off], mem[off + 1]])
            }

            // IO Registers
            // LCD I/O Interrupts and Status: https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus
            0x04000004 => self.gpu.get_dispstat7(), // DISPSTAT - General LCD Status
            0x04000006 => self.gpu.get_vcount(),    // VCOUNT - Vertical Counter

            // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
            0x040000BA => self.dma.read_cnt(4), // DMA0CNT_H - DMA 0 Control
            0x040000C6 => self.dma.read_cnt(5), // DMA1CNT_H - DMA 1 Control
            0x040000D2 => self.dma.read_cnt(6), // DMA2CNT_H - DMA 2 Control
            0x040000DE => self.dma.read_cnt(7), // DMA3CNT_H - DMA 3 Control

            // GBA Timers: https://problemkaputt.de/gbatek.htm#gbatimers
            0x04000100 => self.nds_timing.read_lo(0), // TM0CNT_L - Timer 0 Counter/Reload
            0x04000102 => self.nds_timing.read_hi(0), // TM0CNT_H - Timer 0 Control
            0x04000104 => self.nds_timing.read_lo(1), // TM1CNT_L - Timer 1 Counter/Reload
            0x04000106 => self.nds_timing.read_hi(1), // TM1CNT_H - Timer 1 Control
            0x04000108 => self.nds_timing.read_lo(2), // TM2CNT_L - Timer 2 Counter/Reload
            0x0400010A => self.nds_timing.read_hi(2), // TM2CNT_H - Timer 2 Control
            0x0400010C => self.nds_timing.read_lo(3), // TM3CNT_L - Timer 3 Counter/Reload
            0x0400010E => self.nds_timing.read_hi(3), // TM3CNT_H - Timer 3 Control

            // DS I/O Maps: https://problemkaputt.de/gbatek.htm#dsiomaps
            0x04000128 => self.sio_cnt, // Debug SIOCNT
            // GBA Keypad Input: https://problemkaputt.de/gbatek.htm#gbakeypadinput
            0x04000130 => self.key_input.get_value(), // KEYINPUT - Key Status
            // DS I/O Maps: https://problemkaputt.de/gbatek.htm#dsiomaps
            0x04000134 => self.r_cnt, // Debug RCNT
            // DS Keypad: https://problemkaputt.de/gbatek.htm#dskeypad
            0x04000136 => self.ext_key_in.get_value(), // EXTKEYIN - Key X/Y Input
            // DS Real-Time Clock (RTC): https://problemkaputt.de/gbatek.htm#dsrealtimeclockrtc
            0x04000138 => self.rtc.read(), // RTC - Real Time Clock Bus

            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000180 => self.ipc.read_sync(Side::Arm7), // IPCSYNC - IPC Synchronize Register
            0x04000184 => self.ipc.read_cnt(Side::Arm7),  // IPCFIFOCNT - IPC Fifo Control Register

            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x040001A0 => self.cart.get_auxspicnt(), // AUXSPICNT - Gamecard ROM and SPI Control
            0x040001A2 => self.cart.read_auxspidata().into(), // AUXSPIDATA - Gamecard SPI Bus Data/Strobe

            // DS Serial Peripheral Interface Bus (SPI): https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi
            0x040001C0 => self.spi.get_spicnt(), // SPICNT - SPI Bus Control/Status Register
            0x040001C2 => self.spi.read_spidata().into(), // SPIDATA - SPI Bus Data/Strobe Register

            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x04000208 => self.int7_reg.ime as u16, // IME - Interrupt Master Enable Register
            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            0x04000300 => self.postflg7.into(), // POSTFLG - Post Boot Flag
            0x04000304 => self.pow_cnt2.get_value(), // POWCNT2 - Sound/Wifi Power Control Register

            0x04001080 => 0,
            // DS I/O Maps: https://problemkaputt.de/gbatek.htm#dsiomaps
            0x04004700 => 0, // SNDEXCNT - DSi sound extension control (mapped even in DS mode)

            // Wi-Fi ports and RAM (mirrors above 0x04810000 read as 0)
            // DS Wifi I/O Map: https://problemkaputt.de/gbatek.htm#dswifiiomap
            0x04800000..0x04810000 => self.wifi.read(address), // Wifi WS0/WS1 region (Wifi ports + 8K Wifi RAM)
            // DS I/O Maps: https://problemkaputt.de/gbatek.htm#dsiomaps
            0x04810000..0x04900000 => 0, // unused I/O (reads 00h)

            // GPU
            0x06000000..0x07000000 => self.gpu.read_arm7_u16(address),

            // GBA Slot ROM
            GBA_ROM_START.. => 0xFFFF,

            // Unused IO region
            // DS Sound Channels 0..15: https://problemkaputt.de/gbatek.htm#dssoundchannels015
            0x04000400..0x04000500 => 0, // SOUND0CNT - Sound Channel 0 Control .. SOUND15LEN - Sound Channel 15 Length

            _ => {
                #[cfg(feature = "log")]
                log::warn!("ARM7: Unrecognized halfword read from ${address:08X}");
                0
            }
        }
    }

    /// ARM7 8-bit read (RAM, BIOS, VRAM and the byte-accessible I/O ports).
    pub fn arm7_read_byte(&mut self, address: u32) -> u8 {
        if (0x0400_0400..0x0400_0520).contains(&address) {
            // SOUND0CNT - Sound Channel 0 Control .. SNDCAP1LEN - Sound Capture 1 Length
            return self.spu.read8(address);
        }
        #[allow(non_contiguous_range_endpoints)]
        match address {
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000180 => self.ipc.read_sync(Side::Arm7) as u8, // IPCSYNC - IPC Synchronize Register
            0x04000181 => (self.ipc.read_sync(Side::Arm7) >> 8) as u8, // IPCSYNC - IPC Synchronize Register (byte 1)
            0x04000184 => self.ipc.read_cnt(Side::Arm7) as u8, // IPCFIFOCNT - IPC Fifo Control Register
            0x04000185 => (self.ipc.read_cnt(Side::Arm7) >> 8) as u8, // IPCFIFOCNT - IPC Fifo Control Register (byte 1)
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let off = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[off]
            }

            // Shared WRAM (per WRAMCNT) and ARM7 WRAM
            SHARED_WRAM_START..IO_REGS_START => {
                let (private, off) = self.arm7_wram_offset(address);
                let mem = if private { &self.arm7_wram } else { &self.shared_wram };
                mem[off]
            }

            // Direct IO byte reads
            // DS Real-Time Clock (RTC): https://problemkaputt.de/gbatek.htm#dsrealtimeclockrtc
            0x04000138 => self.rtc.read() as u8, // RTC - Real Time Clock Bus
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x040001A0 => self.cart.get_auxspicnt() as u8, // AUXSPICNT - Gamecard ROM and SPI Control
            0x040001A1 => (self.cart.get_auxspicnt() >> 8) as u8, // AUXSPICNT - Gamecard ROM and SPI Control (byte 1)
            0x040001A2 => self.cart.read_auxspidata(), // AUXSPIDATA - Gamecard SPI Bus Data/Strobe
            // DS Serial Peripheral Interface Bus (SPI): https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi
            0x040001C2 => self.spi.read_spidata(), // SPIDATA - SPI Bus Data/Strobe Register
            0x04000218 | 0x0400021C => 0,          // DSi IE2 | DSi IF2
            // DS Memory Control - WRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolwram
            0x04000241 => self.wram_cnt & 0x3, // WRAMSTAT - WRAM Bank Status
            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            0x04000300 => self.postflg7, // POSTFLG - Post Boot Flag
            // DS Wifi I/O Map: https://problemkaputt.de/gbatek.htm#dswifiiomap
            0x04800000..0x04810000 => (self.wifi.read(address & !1) >> ((address & 1) * 8)) as u8, // Wifi WS0/WS1 region (Wifi ports + 8K Wifi RAM)

            // BIOS (0x00000000..0x00003FFF)
            ..0x4000 => {
                let pc = self.arm7.get_pc();

                // BIOS protection checks
                if pc > 0x4000 || address < self.bios_prot && pc > self.bios_prot {
                    return 0xFF; // protected BIOS reads
                }

                self.arm7_bios[address as usize]
            }

            0x06000000..0x07000000 => self.gpu.read_arm7_u8(address), // VRAM banks C/D mapped to ARM7

            // Default case
            _ => {
                #[cfg(feature = "log")]
                log::warn!("ARM7: Unrecognized byte read from ${address:08X}");
                0
            }
        }
    }
}
