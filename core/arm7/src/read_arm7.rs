// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::emulator::{Emulator, Interrupt};
use mem_const::*;

impl Emulator {
    /// ARM7 read 32-bit word
    ///
    /// FIXME: return Result?
    pub fn arm7_read_word(&mut self, address: u32) -> u32 {
        match address {
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let off = (address & MAIN_RAM_MASK) as usize;
                u32::from_le_bytes(self.main_ram[off..off + 4].try_into().unwrap())
            }
            SHARED_WRAM_START..ARM7_WRAM_START => {
                let off = match self.wramcnt {
                    0 => address & ARM7_WRAM_MASK,    // Mirror to ARM7 WRAM
                    1 => address & 0x3FFF,            // First half
                    2 => (address & 0x3FFF) + 0x4000, // Second half
                    3 => address & 0x7FFF,            // Entire 32 KB
                    _ => return 0,                    // TODO: Log trancing error
                } as usize;
                u32::from_le_bytes(self.arm7_wram[off..off + 4].try_into().unwrap())
            }

            0x04000120 => 0,
            0x04000180 => self.ipcsync_nds7.read().into(),
            0x040001A4 => self.cart.get_romctrl(),
            0x040001C0 => (self.spi.get_spicnt() | (self.spi.read_spidata() << 16) as u16).into(),
            0x04000208 => self.int7_reg_ime as u32,
            0x04000210 => self.int7_reg_ie,
            0x04000214 => self.int7_reg_if,
            0x04100000 => {
                let word: u32 = self.fifo9.read_queue();
                if self.fifo9.request_empty_irq {
                    self.request_interrupt9(Interrupt::IPC_FIFO_EMPTY);
                    self.fifo9.request_empty_irq = false;
                }
                word
            }
            0x04100010 => self.cart.get_output(),

            ..0x4000 => {
                let arm7_pc = self.arm7.get_pc();
                if arm7_pc > 0x4000 || (address < self.biosprot && arm7_pc > self.biosprot) {
                    return 0xFFFF_FFFF;
                }

                let start = address as usize;
                let end = (address + 4) as usize;
                u32::from_le_bytes(self.arm7_bios[start..end].try_into().unwrap())
            }
            0x04000400..0x04000500 => 0,
            0x06000000..0x07000000 => self.gpu.read_arm7_u32(address),
            GBA_ROM_START.. => 0xFFFF_FFFF,

            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!("ARM7: Unrecognized word read from ${address:08X}");
                0
            }
        }
    }

    /// ARM7 read 16-bit halfword
    pub fn arm7_read_halfword(&mut self, address: u32) -> u16 {
        match address {
            // BIOS (0x00000000..0x00003FFF)
            ..0x4000 => {
                let pc = self.arm7.get_pc();
                if pc > 0x4000 {
                    return 0xFFFF;
                }
                if address < self.biosprot && pc > self.biosprot {
                    return 0xFFFF;
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

            // Shared WRAM
            SHARED_WRAM_START..ARM7_WRAM_START => {
                let off = match self.wramcnt {
                    0 => address & ARM7_WRAM_MASK,
                    1 => address & 0x3FFF,
                    2 => (address & 0x3FFF) + 0x4000,
                    3 => address & 0x7FFF,
                    _ => return 0,
                } as usize;

                u16::from_le_bytes(self.arm7_wram[off..off + 2].try_into().unwrap())
            }

            // ARM7 WRAM
            ARM7_WRAM_START..IO_REGS_START => {
                let off = (address & 0xFFFF) as usize;
                u16::from_le_bytes(self.arm7_wram[off..off + 2].try_into().unwrap())
            }

            // IO Registers
            0x04000004 => self.gpu.get_dispstat7(),
            0x04000006 => self.gpu.get_vcount(),

            0x040000BA => self.dma.read_cnt(4),
            0x040000C6 => self.dma.read_cnt(5),
            0x040000D2 => self.dma.read_cnt(6),
            0x040000DE => self.dma.read_cnt(7),

            0x04000100 => self.timers.read_lo(0),
            0x04000102 => self.timers.read_hi(0),
            0x04000104 => self.timers.read_lo(1),
            0x04000106 => self.timers.read_hi(1),
            0x04000108 => self.timers.read_lo(2),
            0x0400010A => self.timers.read_hi(2),
            0x0400010C => self.timers.read_lo(3),
            0x0400010E => self.timers.read_hi(3),

            0x04000128 => self.siocnt,
            0x04000130 => self.key_input.get_value(),
            0x04000134 => self.rcnt,
            0x04000136 => self.ext_key_in.get_value(),
            0x04000138 => self.rtc.read(),

            0x04000180 => self.ipcsync_nds7.read(),
            0x04000184 => self.fifo7.read_cnt(),

            0x040001A0 => self.cart.get_auxspicnt(),
            0x040001A2 => self.cart.read_auxspidata().into(),

            0x040001C0 => self.spi.get_spicnt(),
            0x040001C2 => self.spi.read_spidata().into(),

            0x04000208 => self.int7_reg_ime.into(),
            0x04000300 => self.postflg7.into(),
            0x04000304 => self.pow_cnt2.get_value(),
            0x04000500 => self.spu.get_soundcnt(),
            0x04000504 => self.spu.get_soundbias(),
            0x04000508 => self.spu.get_sndcap0() as u16 | ((self.spu.get_sndcap1() as u16) << 8),

            0x04001080 => 0,
            0x04004700 => 0,

            0x0480815C => self.wifi.get_w_bb_read(),
            0x0480815E => self.wifi.get_w_bb_busy() as u16,
            0x04808180 => self.wifi.get_w_rf_busy() as u16,

            // WiFi block
            0x04800000..0x04900000 => 0,

            // GPU
            0x06000000..0x07000000 => self.gpu.read_arm7_u16(address),

            // GBA Slot ROM
            GBA_ROM_START.. => 0xFFFF,

            // Unused IO region
            0x04000400..0x04000500 => 0,

            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!("ARM7: Unrecognized halfword read from ${address:08X}");
                0
            }
        }
    }

    pub fn arm7_read_byte(&mut self, address: u32) -> u8 {
        #[allow(non_contiguous_range_endpoints)]
        match address {
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let off = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[off]
            }

            // ARM7 WRAM
            ARM7_WRAM_START..IO_REGS_START => {
                let off = (address & ARM7_WRAM_MASK) as usize;
                self.arm7_wram[off]
            }

            // Shared WRAM
            SHARED_WRAM_START..ARM7_WRAM_START => {
                let off = match self.wramcnt {
                    0 => address & ARM7_WRAM_MASK,    // Mirror to ARM7 WRAM
                    1 => address & 0x3FFF,            // First half
                    2 => (address & 0x3FFF) + 0x4000, // Second half
                    3 => address & 0x7FFF,            // Entire 32 KB
                    _ => return 0,
                } as usize;

                self.arm7_wram[off]
            }

            // Direct IO byte reads
            0x04000138 => self.rtc.read() as u8,
            0x040001C2 => self.spi.read_spidata(),
            0x04000218 => 0, // DSi IE2
            0x0400021C => 0, // DSi IF2
            0x04000241 => self.wramcnt & 0x3,
            0x04000300 => self.postflg7,
            0x04000501 => (self.spu.get_soundcnt() >> 8) as u8,
            0x04000508 => self.spu.get_sndcap0(),
            0x04000509 => self.spu.get_sndcap1(),

            // BIOS (0x00000000..0x00003FFF)
            ..0x4000 => {
                let pc = self.arm7.get_pc();

                // BIOS protection checks
                if pc > 0x4000 || address < self.biosprot && pc > self.biosprot {
                    return 0xFF;
                }

                self.arm7_bios[address as usize]
            }

            // SPU channel region
            0x04000400..0x04000500 => self.spu.read_channel_byte(address),

            // Default case
            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!("ARM7: Unrecognized byte read from ${address:08X}");
                0
            }
        }
    }
}
