// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
use ds_interrupt::Interrupt;
use ds_mem_addr::*;
use ds_peripherals::ipc::Side;

use super::Emulator;

impl Emulator {
    /// ARM7 32-bit write: the ARM7 memory map.
    ///
    /// ```text
    /// from: write_word(Arm7) / ARM7 DMA (dma_handle_event)
    ///   |
    ///   v
    /// arm7_write_word(address, word)
    ///   0x0400_0400..0x0400_051F  SPU registers  -> spu.write32
    ///   main RAM / shared WRAM / ARM7 WRAM
    ///   DMA 4-7 -> dma_write_len_cnt, timers 0-3, IPC (may raise IRQ),
    ///   cartridge (AUXSPI, ROMCTRL, KEY2 seeds), IE/IF/IME,
    ///   POWCNT2 -> wifi.set_power_cnt, BIOS protection
    ///   0x0480_0000..0x0480_FFFF  Wi-Fi -> wifi.write -> raise_wifi_irq
    ///   VRAM banks C/D mapped to the ARM7
    /// ```
    /// # Panics
    #[expect(clippy::match_same_arms)]
    pub fn arm7_write_word(&mut self, address: u32, word: u32) {
        // DS Sound Channels 0..15: https://problemkaputt.de/gbatek.htm#dssoundchannels015
        if (0x0400_0400..0x0400_0520).contains(&address) {
            // SOUND0CNT - Sound Channel 0 Control .. SNDCAP1LEN - Sound Capture 1 Length
            return self.spu.write32(address, word);
        }
        match address {
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let off = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[off..off + 4].copy_from_slice(&word.to_le_bytes());
            }

            // Shared WRAM (per WRAMCNT) and ARM7 WRAM
            SHARED_WRAM_START..IO_REGS_START => {
                let (private, off) = self.arm7_wram_offset(address);
                let mem = if private { &mut self.arm7_wram } else { &mut self.shared_wram };
                mem[off..off + 4].copy_from_slice(&word.to_le_bytes());
            }

            // Direct IO write (word)
            // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
            0x040000B0 => self.dma.write_source(4, word), // DMA0SAD - DMA 0 Source Address
            0x040000B4 => self.dma.write_dest(4, word),   // DMA0DAD - DMA 0 Destination Address
            0x040000B8 => self.dma_write_len_cnt(4, word), // DMA0CNT_L - DMA 0 Word Count

            0x040000BC => self.dma.write_source(5, word), // DMA1SAD - DMA 1 Source Address
            0x040000C0 => self.dma.write_dest(5, word),   // DMA1DAD - DMA 1 Destination Address
            0x040000C4 => self.dma_write_len_cnt(5, word), // DMA1CNT_L - DMA 1 Word Count

            0x040000C8 => self.dma.write_source(6, word), // DMA2SAD - DMA 2 Source Address
            0x040000CC => self.dma.write_dest(6, word),   // DMA2DAD - DMA 2 Destination Address
            0x040000D0 => self.dma_write_len_cnt(6, word), // DMA2CNT_L - DMA 2 Word Count

            0x040000D4 => self.dma.write_source(7, word), // DMA3SAD - DMA 3 Source Address
            0x040000D8 => self.dma.write_dest(7, word),   // DMA3DAD - DMA 3 Destination Address
            0x040000DC => self.dma_write_len_cnt(7, word), // DMA3CNT_L - DMA 3 Word Count

            // Timers: LO/HIGH packed in one word
            // GBA Timers: https://problemkaputt.de/gbatek.htm#gbatimers
            0x04000100 => {
                // TM0CNT_L - Timer 0 Counter/Reload
                self.nds_timing.write_lo((word & 0xFFFF) as u16, 0); // TM0CNT_L (lower halfword)
                self.nds_timing.write_hi((word >> 16) as u16, 0);
            }
            0x04000104 => {
                // TM1CNT_L - Timer 1 Counter/Reload
                self.nds_timing.write_lo((word & 0xFFFF) as u16, 1); // TM1CNT_L (lower halfword)
                self.nds_timing.write_hi((word >> 16) as u16, 1);
            }
            0x04000108 => {
                // TM2CNT_L - Timer 2 Counter/Reload
                self.nds_timing.write_lo((word & 0xFFFF) as u16, 2); // TM2CNT_L (lower halfword)
                self.nds_timing.write_hi((word >> 16) as u16, 2);
            }
            0x0400010C => {
                // TM3CNT_L - Timer 3 Counter/Reload
                self.nds_timing.write_lo((word & 0xFFFF) as u16, 3); // TM3CNT_L (lower halfword)
                self.nds_timing.write_hi((word >> 16) as u16, 3);
            }

            0x04000120 | 0x04000128 => {} // SIODATA32 ignored | write ignored

            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000180 => {
                // IPCSYNC - IPC Synchronize Register
                let irq = self.ipc.write_sync(Side::Arm7, word as u16, 0xFFFF); // full IPCSYNC halfword
                self.raise_ipc(irq);
            }

            0x04000184 => {
                // IPCFIFOCNT - IPC Fifo Control Register
                for irq in self.ipc.write_cnt(Side::Arm7, word as u16) {
                    self.raise_ipc(irq);
                }
            }
            0x04000188 => {
                // IPCFIFOSEND - IPC Send Fifo
                #[cfg(feature = "log")]
                log::debug!(target: "ipc", "Arm7 send {word:08X}");
                let irq = self.ipc.send(Side::Arm7, word);
                self.raise_ipc(irq);
            }

            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x040001A0 => {
                // AUXSPICNT - Gamecard ROM and SPI Control
                self.cart.set_auxspicnt(word as u16);
                self.cart.set_auxspidata((word >> 16) as u8);
            }
            0x040001A4 => self.cart.set_romctrl(word), // ROMCTRL - Gamecard Bus ROMCTRL
            0x04100010 if self.arm7_has_cart_rights() => self.cart_write_data(word), // Gamecard bus data out
            0x040001B0 => self.cart.set_lo_key2_seed0(word), // Encryption Seed 0 Lower 32bit
            0x040001B4 => self.cart.set_lo_key2_seed1(word), // Encryption Seed 1 Lower 32bit

            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x04000208 => self.int7_reg.ime = word & 0x1, // IME - Interrupt Master Enable Register
            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            0x04000304 => {
                // POWCNT2 - Sound/Wifi Power Control Register
                self.pow_cnt2.set_value(word as u16);
                self.wifi.set_power_cnt(self.pow_cnt2.wifi);
            }
            // DS Wifi I/O Map: https://problemkaputt.de/gbatek.htm#dswifiiomap
            0x04800000..0x04810000 => {
                // Wifi WS0/WS1 region (Wifi ports + 8K Wifi RAM)
                self.wifi.write(address, word as u16);
                self.wifi.write(address + 2, (word >> 16) as u16);
                self.raise_wifi_irq();
            }
            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x04000210 => self.int7_reg.irq_enable = word, // IE - Interrupt Enable
            0x04000214 => self.int7_reg.irq_flags &= !word, // IF - Interrupt Request Flags

            0x04000218 => {} // ignore
            // DS Memory Control - BIOS: https://problemkaputt.de/gbatek.htm#dsmemorycontrolbios
            0x04000308 => self.bios_prot = word & !1, // BIOSPROT - Bios-data-read-protection address
            // DS Sound Capture: https://problemkaputt.de/gbatek.htm#dssoundcapture
            0x04000510 => {} // SNDCAP0DAD - Sound Capture 0 Destination Address
            0x04000518 => {} // SNDCAP1DAD - Sound Capture 1 Destination Address

            // SPU channel write region

            // GPU VRAM write (ARM7)
            0x06000000..0x07000000 => self.gpu.write_arm7_u32(address, word),

            // Default
            _ => {
                #[cfg(feature = "log")]
                log::warn!("(7) Unrecognized word write {:08X} -> {:08X}", word, address);
            }
        }
    }

    /// ARM7 16-bit write (same map as [`Self::arm7_write_word`]; adds
    /// SPICNT/SPIDATA -> spi bus (touch screen, firmware, power), RTC,
    /// POSTFLG and the Wi-Fi ports).
    #[expect(clippy::match_same_arms)]
    pub fn arm7_write_halfword(&mut self, address: u32, halfword: u16) {
        // DS Sound Channels 0..15: https://problemkaputt.de/gbatek.htm#dssoundchannels015
        if (0x0400_0400..0x0400_0520).contains(&address) {
            // SOUND0CNT - Sound Channel 0 Control .. SNDCAP1LEN - Sound Capture 1 Length
            return self.spu.write16(address, halfword);
        }
        match address {
            0x06000000..0x07000000 => self.gpu.write_arm7_u16(address, halfword),
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let off = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[off..off + 2].copy_from_slice(&halfword.to_le_bytes());
            }

            // Shared WRAM (per WRAMCNT) and ARM7 WRAM
            SHARED_WRAM_START..IO_REGS_START => {
                let (private, off) = self.arm7_wram_offset(address);
                let mem = if private { &mut self.arm7_wram } else { &mut self.shared_wram };
                mem[off..off + 2].copy_from_slice(&halfword.to_le_bytes());
            }

            // IO register halfword writes
            // LCD I/O Interrupts and Status: https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus
            0x04000004 => self.gpu.set_dispstat7(halfword), // DISPSTAT - General LCD Status

            // DMA0-3 (ARM7 channels 4-7) SAD/DAD/CNT halfword sub-writes.
            // Previously only the control halfword was wired here, so
            // source/destination addresses set via halfword stores (STRH)
            // were silently dropped and every transfer read/wrote address 0.
            // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
            0x040000B0 => self.dma.write_source_lo(4, halfword), // DMA0SAD - DMA 0 Source Address
            0x040000B2 => self.dma.write_source_hi(4, halfword), // DMA0SAD - DMA 0 Source Address (upper halfword)
            0x040000B4 => self.dma.write_dest_lo(4, halfword), // DMA0DAD - DMA 0 Destination Address
            0x040000B6 => self.dma.write_dest_hi(4, halfword), // DMA0DAD - DMA 0 Destination Address (upper halfword)
            0x040000B8 => self.dma.write_len(4, halfword),     // DMA0CNT_L - DMA 0 Word Count
            0x040000BA => self.dma_write_cnt(4, halfword),     // DMA0CNT_H - DMA 0 Control

            0x040000BC => self.dma.write_source_lo(5, halfword), // DMA1SAD - DMA 1 Source Address
            0x040000BE => self.dma.write_source_hi(5, halfword), // DMA1SAD - DMA 1 Source Address (upper halfword)
            0x040000C0 => self.dma.write_dest_lo(5, halfword), // DMA1DAD - DMA 1 Destination Address
            0x040000C2 => self.dma.write_dest_hi(5, halfword), // DMA1DAD - DMA 1 Destination Address (upper halfword)
            0x040000C4 => self.dma.write_len(5, halfword),     // DMA1CNT_L - DMA 1 Word Count
            0x040000C6 => self.dma_write_cnt(5, halfword),     // DMA1CNT_H - DMA 1 Control

            0x040000C8 => self.dma.write_source_lo(6, halfword), // DMA2SAD - DMA 2 Source Address
            0x040000CA => self.dma.write_source_hi(6, halfword), // DMA2SAD - DMA 2 Source Address (upper halfword)
            0x040000CC => self.dma.write_dest_lo(6, halfword), // DMA2DAD - DMA 2 Destination Address
            0x040000CE => self.dma.write_dest_hi(6, halfword), // DMA2DAD - DMA 2 Destination Address (upper halfword)
            0x040000D0 => self.dma.write_len(6, halfword),     // DMA2CNT_L - DMA 2 Word Count
            0x040000D2 => self.dma_write_cnt(6, halfword),     // DMA2CNT_H - DMA 2 Control

            0x040000D4 => self.dma.write_source_lo(7, halfword), // DMA3SAD - DMA 3 Source Address
            0x040000D6 => self.dma.write_source_hi(7, halfword), // DMA3SAD - DMA 3 Source Address (upper halfword)
            0x040000D8 => self.dma.write_dest_lo(7, halfword), // DMA3DAD - DMA 3 Destination Address
            0x040000DA => self.dma.write_dest_hi(7, halfword), // DMA3DAD - DMA 3 Destination Address (upper halfword)
            0x040000DC => self.dma.write_len(7, halfword),     // DMA3CNT_L - DMA 3 Word Count
            0x040000DE => self.dma_write_cnt(7, halfword),     // DMA3CNT_H - DMA 3 Control

            // GBA Timers: https://problemkaputt.de/gbatek.htm#gbatimers
            0x04000100 => self.nds_timing.write_lo(halfword, 0), // TM0CNT_L - Timer 0 Counter/Reload
            0x04000102 => self.nds_timing.write_hi(halfword, 0), // TM0CNT_H - Timer 0 Control
            0x04000104 => self.nds_timing.write_lo(halfword, 1), // TM1CNT_L - Timer 1 Counter/Reload
            0x04000106 => self.nds_timing.write_hi(halfword, 1), // TM1CNT_H - Timer 1 Control
            0x04000108 => self.nds_timing.write_lo(halfword, 2), // TM2CNT_L - Timer 2 Counter/Reload
            0x0400010A => self.nds_timing.write_hi(halfword, 2), // TM2CNT_H - Timer 2 Control
            0x0400010C => self.nds_timing.write_lo(halfword, 3), // TM3CNT_L - Timer 3 Counter/Reload
            0x0400010E => self.nds_timing.write_hi(halfword, 3), // TM3CNT_H - Timer 3 Control

            // DS I/O Maps: https://problemkaputt.de/gbatek.htm#dsiomaps
            0x04000128 => self.sio_cnt = halfword, // Debug SIOCNT
            0x04000132 => self.write_keycnt(1, halfword), // KEYCNT - Key Interrupt Control
            0x04000134 => self.r_cnt = halfword,   // Debug RCNT

            // DS Real-Time Clock (RTC): https://problemkaputt.de/gbatek.htm#dsrealtimeclockrtc
            0x04000138 => self.rtc.write(halfword, false), // RTC - Real Time Clock Bus

            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000180 => {
                // IPCSYNC - IPC Synchronize Register
                let irq = self.ipc.write_sync(Side::Arm7, halfword, 0xFFFF); // full IPCSYNC halfword
                self.raise_ipc(irq);
            }

            0x04000184 => {
                // IPCFIFOCNT - IPC Fifo Control Register
                for irq in self.ipc.write_cnt(Side::Arm7, halfword) {
                    self.raise_ipc(irq);
                }
            }

            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x040001A0 => self.cart.set_auxspicnt(halfword), // AUXSPICNT - Gamecard ROM and SPI Control

            0x040001A2 => {
                // AUXSPIDATA - Gamecard SPI Bus Data/Strobe
                // #[cfg(feature = "log")]
                // log::info!("AUXSPIDATA: {:04X}", halfword);
                self.cart.set_auxspidata((halfword & 0xFF) as u8);
            }

            0x040001B8 => self.cart.set_hi_key2_seed0(halfword.into()), // Encryption Seed 0 Upper 7bit (bit7-15 unused)
            0x040001BA => self.cart.set_hi_key2_seed1(halfword.into()), // Encryption Seed 1 Upper 7bit (bit7-15 unused)

            // DS Serial Peripheral Interface Bus (SPI): https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi
            0x040001C0 => self.spi.set_spicnt(halfword), // SPICNT - SPI Bus Control/Status Register
            0x040001C2 => {
                // SPIDATA - SPI Bus Data/Strobe Register
                self.spi.set_frame_pos(self.system_timestamp - self.frame_start);
                if self.spi.write_spidata((halfword & 0xFF) as u8) {
                    // SPIDATA (bits 0-7)
                    self.request_interrupt7(Interrupt::Spi);
                }
            }

            0x04000206 => self.wifi_wait_cnt = halfword & 0x3F, // WIFIWAITCNT (bits 0-5)

            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x04000208 => self.int7_reg.ime = (halfword & 0x1) as u32, // IME - Interrupt Master Enable Register

            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            0x04000300 => self.postflg7 = (halfword & 1) as u8, // POSTFLG - Post Boot Flag
            0x04000304 => {
                // POWCNT2 - Sound/Wifi Power Control Register
                self.pow_cnt2.set_value(halfword);
                self.wifi.set_power_cnt(self.pow_cnt2.wifi);
            }

            // DS Sound Capture: https://problemkaputt.de/gbatek.htm#dssoundcapture
            0x04000514 => {} // SNDCAP0LEN - Sound Capture 0 Length
            0x0400051C => {} // SNDCAP1LEN - Sound Capture 1 Length

            // Debug port (DS Lite firmware)
            0x04001080 => {}

            // Wi-Fi ports and RAM
            // DS Wifi I/O Map: https://problemkaputt.de/gbatek.htm#dswifiiomap
            0x04800000..0x04810000 => {
                // Wifi WS0/WS1 region (Wifi ports + 8K Wifi RAM)
                self.wifi.write(address, halfword);
                self.raise_wifi_irq();
            }
            // DS I/O Maps: https://problemkaputt.de/gbatek.htm#dsiomaps
            0x04810000..0x04900000 => {} // unused I/O (reads 00h)

            // Default
            _ => {
                #[cfg(feature = "log")]
                log::warn!("(7) Unrecognized halfword write {:04X} -> {:08X}", halfword, address);
            }
        }
    }

    /// ARM7 8-bit write (RAM, IPCSYNC, RTC, AUXSPI, cartridge command
    /// bytes, SPI data, IME, HALTCNT...).
    pub fn arm7_write_byte(&mut self, address: u32, byte: u8) {
        // DS Sound Channels 0..15: https://problemkaputt.de/gbatek.htm#dssoundchannels015
        if (0x0400_0400..0x0400_0520).contains(&address) {
            // SOUND0CNT - Sound Channel 0 Control .. SNDCAP1LEN - Sound Capture 1 Length
            return self.spu.write8(address, byte);
        }
        match address {
            // DS Inter Process Communication (IPC): https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc
            0x04000181 => {
                // IPCSYNC - IPC Synchronize Register (byte 1)
                let irq = self.ipc.write_sync(Side::Arm7, u16::from(byte) << 8, 0xFF00); // IPCSYNC upper byte (output bits 8-11, IRQ bits 13-14)
                self.raise_ipc(irq);
            }
            0x04000180 => {} // IPCSYNC low byte is read-only
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                self.main_ram[(address & MAIN_RAM_MASK) as usize] = byte;
            }

            // Shared WRAM (per WRAMCNT) and ARM7 WRAM
            SHARED_WRAM_START..IO_REGS_START => {
                let (private, off) = self.arm7_wram_offset(address);
                let mem = if private { &mut self.arm7_wram } else { &mut self.shared_wram };
                mem[off] = byte;
            }

            // IO register byte writes
            // DS Real-Time Clock (RTC): https://problemkaputt.de/gbatek.htm#dsrealtimeclockrtc
            0x04000138 => self.rtc.write(byte as u16, true), // RTC - Real Time Clock Bus

            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            0x040001A0 => self.cart.set_lo_auxspicnt(byte), // AUXSPICNT - Gamecard ROM and SPI Control
            0x040001A1 => self.cart.set_hi_auxspicnt(byte), // AUXSPICNT - Gamecard ROM and SPI Control (byte 1)
            0x040001A2 => self.cart.set_auxspidata(byte), // AUXSPIDATA - Gamecard SPI Bus Data/Strobe

            // Cart command sequence
            0x040001A8..=0x040001AF => self // Gamecard bus 8-byte Command Out
                .cart
                .receive_command(byte, (address - 0x040001A8) as usize), // Gamecard bus 8-byte Command Out

            // DS Serial Peripheral Interface Bus (SPI): https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi
            0x040001C2 => {
                // SPIDATA - SPI Bus Data/Strobe Register
                self.spi.set_frame_pos(self.system_timestamp - self.frame_start);
                if self.spi.write_spidata(byte) {
                    self.request_interrupt7(Interrupt::Spi);
                }
            }

            // DS Interrupts: https://problemkaputt.de/gbatek.htm#dsinterrupts
            0x04000208 => self.int7_reg.ime = (byte & 0x1) as u32, // IME - Interrupt Master Enable Register

            // DS Power Control: https://problemkaputt.de/gbatek.htm#dspowercontrol
            0x04000300 => self.postflg7 = byte & 0x1, // POSTFLG - Post Boot Flag

            0x04000301 => {
                // HALTCNT - Low Power Mode Control
                if byte == 0x80 {
                    // ARM7 halt request
                    self.arm7.halt();
                } else if byte == 0xC0 {
                    // HALTCNT C0h: sleep
                    self.enter_sleep();
                } else {
                    #[cfg(feature = "log")]
                    log::error!("Unrecognized HALTCNT value {:02X} for ARM7", byte);
                    // original code exits, but emulator should not crash
                }
            }

            // SPU channel writes

            // GPU VRAM (ARM7 writes allowed)
            0x06000000..0x07000000 => self.gpu.write_arm7_u8(address, byte),

            // Ignore BIOS writes
            0x00000000..0x00004000 => {}

            // Default
            _ => {
                #[cfg(feature = "log")]
                log::warn!("(7) Unrecognized byte write {:02X} -> {:08X}", byte, address);
            }
        }
    }
}
