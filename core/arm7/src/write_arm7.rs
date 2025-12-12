// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
use crate::emulator::{Emulator, Interrupt};
use mem_const::*;

impl Emulator {
    pub fn arm7_write_word(&mut self, address: u32, word: u32) {
        // Debug print (kept from original C++)
        if address == 0x027E0014 {
            #[cfg(feature = "tracing")]
            tracing::info!("(7) Write of {:08X} to {:08X}", word, address);
        }

        match address {
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let off = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[off..off + 4].copy_from_slice(&word.to_le_bytes())
            }

            // Shared WRAM
            SHARED_WRAM_START..ARM7_WRAM_START => {
                let off = match self.wramcnt {
                    0 => address & ARM7_WRAM_MASK,    // Mirror to ARM7 WRAM
                    1 => address & 0x3FFF,            // First half
                    2 => (address & 0x3FFF) + 0x4000, // Second half
                    3 => address & 0x7FFF,            // Entire 32 KB
                    _ => return,
                } as usize;

                self.arm7_wram[off..off + 4].copy_from_slice(&word.to_le_bytes())
            }

            // ARM7 WRAM
            ARM7_WRAM_START..IO_REGS_START => {
                let off = (address & ARM7_WRAM_MASK) as usize;
                self.arm7_wram[off..off + 4].copy_from_slice(&word.to_le_bytes())
            }

            // Direct IO write (word)
            0x040000B0 => self.dma.write_source(4, word),
            0x040000B4 => self.dma.write_dest(4, word),
            0x040000B8 => self.dma.write_len_cnt(4, word),

            0x040000BC => self.dma.write_source(5, word),
            0x040000C0 => self.dma.write_dest(5, word),
            0x040000C4 => self.dma.write_len_cnt(5, word),

            0x040000C8 => self.dma.write_source(6, word),
            0x040000CC => self.dma.write_dest(6, word),
            0x040000D0 => self.dma.write_len_cnt(6, word),

            0x040000D4 => self.dma.write_source(7, word),
            0x040000D8 => self.dma.write_dest(7, word),
            0x040000DC => self.dma.write_len_cnt(7, word),

            // Timers: LO/HIGH packed in one word
            0x04000100 => {
                self.timers.write_lo((word & 0xFFFF) as u16, 0);
                self.timers.write_hi((word >> 16) as u16, 0)
            }
            0x04000104 => {
                self.timers.write_lo((word & 0xFFFF) as u16, 1);
                self.timers.write_hi((word >> 16) as u16, 1)
            }
            0x04000108 => {
                self.timers.write_lo((word & 0xFFFF) as u16, 2);
                self.timers.write_hi((word >> 16) as u16, 2)
            }
            0x0400010C => {
                self.timers.write_lo((word & 0xFFFF) as u16, 3);
                self.timers.write_hi((word >> 16) as u16, 3)
            }

            0x04000120 => {} // SIODATA32 ignored
            0x04000128 => {} // write ignored

            0x04000180 => {
                self.ipcsync_nds7.write(word as u16);
                self.ipcsync_nds9.receive_input(word as u16);

                // IPCSYNC interrupt
                if (word & (1 << 13)) != 0 && self.ipcsync_nds9.irq_enable {
                    self.request_interrupt9(Interrupt::IPCSYNC)
                }
            }

            0x04000188 => {
                self.fifo9.write_queue(word);
                if self.fifo9.request_nempty_irq {
                    self.request_interrupt9(Interrupt::IPC_FIFO_NEMPTY);
                    self.fifo9.request_nempty_irq = false;
                }
            }

            0x040001A4 => self.cart.set_romctrl(word),
            0x040001B0 => self.cart.set_lo_key2_seed0(word),
            0x040001B4 => self.cart.set_lo_key2_seed1(word),

            0x04000208 => self.int7_reg_ime = (word & 1) as u8,
            0x04000210 => self.int7_reg_ie = word,
            0x04000214 => self.int7_reg_if &= !word,

            0x04000218 => {} // ignore
            0x04000308 => self.biosprot = word & !1,

            0x04000500 => self.spu.set_soundcnt((word & 0xFFFF) as u16),
            0x04000510 => {}
            0x04000518 => {}

            // SPU channel write region
            0x04000400..0x04000500 => self.spu.write_channel_word(address, word),

            // GPU VRAM write (ARM7)
            0x06000000..0x07000000 => self.gpu.write_arm7_u32(address, word),

            // Default
            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!(
                    "(7) Unrecognized word write {:08X} -> {:08X}",
                    word,
                    address
                );
            }
        }
    }

    pub fn arm7_write_halfword(&mut self, address: u32, halfword: u16) {
        match address {
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                let off = (address & MAIN_RAM_MASK) as usize;
                self.main_ram[off..off + 2].copy_from_slice(&halfword.to_le_bytes())
            }

            // Shared WRAM
            SHARED_WRAM_START..ARM7_WRAM_START => {
                let off = match self.wramcnt {
                    0 => address & ARM7_WRAM_MASK,    // Mirror to ARM7 WRAM
                    1 => address & 0x3FFF,            // First half
                    2 => (address & 0x3FFF) + 0x4000, // Second half
                    3 => address & 0x7FFF,            // Entire 32 KB
                    _ => return,
                } as usize;

                self.arm7_wram[off..off + 2].copy_from_slice(&halfword.to_le_bytes())
            }

            // ARM7 WRAM
            ARM7_WRAM_START..IO_REGS_START => {
                let off = (address & ARM7_WRAM_MASK) as usize;
                self.arm7_wram[off..off + 2].copy_from_slice(&halfword.to_le_bytes())
            }

            // IO register halfword writes
            0x04000004 => self.gpu.set_dispstat7(halfword),

            0x040000BA => self.dma.write_cnt(4, halfword),
            0x040000C6 => self.dma.write_cnt(5, halfword),
            0x040000D2 => self.dma.write_cnt(6, halfword),
            0x040000DE => self.dma.write_cnt(7, halfword),

            0x04000100 => self.timers.write_lo(halfword, 0),
            0x04000102 => self.timers.write_hi(halfword, 0),
            0x04000104 => self.timers.write_lo(halfword, 1),
            0x04000106 => self.timers.write_hi(halfword, 1),
            0x04000108 => self.timers.write_lo(halfword, 2),
            0x0400010A => self.timers.write_hi(halfword, 2),
            0x0400010C => self.timers.write_lo(halfword, 3),
            0x0400010E => self.timers.write_hi(halfword, 3),

            0x04000128 => self.siocnt = halfword,
            0x04000134 => self.rcnt = halfword,

            0x04000138 => self.rtc.write(halfword, false),

            0x04000180 => {
                self.ipcsync_nds7.write(halfword);
                self.ipcsync_nds9.receive_input(halfword);

                // Trigger IPCSYNC interrupt if enabled
                if (halfword & (1 << 13)) != 0 && self.ipcsync_nds9.irq_enable {
                    self.request_interrupt9(Interrupt::IPCSYNC)
                }
            }

            0x04000184 => {
                self.fifo7.write_cnt(halfword);

                // FIFO empty IRQ
                if self.fifo7.request_empty_irq {
                    self.request_interrupt7(Interrupt::IPC_FIFO_EMPTY);
                    self.fifo7.request_empty_irq = false;
                }

                // FIFO non-empty IRQ
                if self.fifo7.request_nempty_irq {
                    self.request_interrupt7(Interrupt::IPC_FIFO_NEMPTY);
                    self.fifo7.request_nempty_irq = false;
                }
            }

            0x040001A0 => self.cart.set_auxspicnt(halfword),

            0x040001A2 => {
                #[cfg(feature = "tracing")]
                tracing::info!("AUXSPIDATA: {:04X}", halfword);
                self.cart.set_auxspidata((halfword & 0xFF) as u8)
            }

            0x040001B8 => self.cart.set_hi_key2_seed0(halfword.into()),
            0x040001BA => self.cart.set_hi_key2_seed1(halfword.into()),

            0x040001C0 => self.spi.set_spicnt(halfword),
            0x040001C2 => self.spi.write_spidata((halfword & 0xFF) as u8),

            0x04000206 => {} // WIFIWAITCNT TODO

            0x04000208 => self.int7_reg_ime = (halfword & 1) as u8,

            0x04000300 => self.postflg7 = (halfword & 1) as u8,
            0x04000304 => self.pow_cnt2.set_value(halfword),

            0x04000500 => self.spu.set_soundcnt(halfword),
            0x04000504 => self.spu.set_soundbias(halfword),

            0x04000508 => {
                self.spu.set_sndcap0((halfword & 0xFF) as u8);
                self.spu.set_sndcap1((halfword >> 8) as u8)
            }

            0x04000514 => {}
            0x0400051C => {}

            // Debug port (DS Lite firmware)
            0x04001080 => {}

            // WiFi registers
            0x04808036 => self.wifi.set_w_power_us(halfword),
            0x04808158 => self.wifi.set_w_bb_cnt(halfword),
            0x0480815A => self.wifi.set_w_bb_write(halfword),
            0x04808160 => self.wifi.set_w_bb_mode(halfword),
            0x04808168 => self.wifi.set_w_bb_power(halfword),
            0x04808184 => self.wifi.set_w_rf_cnt(halfword),

            // SPU channel writes
            0x04000400..0x04000500 => self.spu.write_channel_halfword(address, halfword),

            // WiFi block (ignored)
            0x04800000..0x04900000 => {}

            // Default
            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!(
                    "(7) Unrecognized halfword write {:04X} -> {:08X}",
                    halfword,
                    address
                );
            }
        }
    }

    pub fn arm7_write_byte(&mut self, address: u32, byte: u8) {
        match address {
            // Main RAM
            MAIN_RAM_START..SHARED_WRAM_START => {
                self.main_ram[(address & MAIN_RAM_MASK) as usize] = byte
            }

            // ARM7 WRAM
            ARM7_WRAM_START..IO_REGS_START => {
                self.arm7_wram[(address & ARM7_WRAM_MASK) as usize] = byte
            }

            // Shared WRAM
            SHARED_WRAM_START..ARM7_WRAM_START => {
                let off = match self.wramcnt {
                    0 => address & ARM7_WRAM_MASK,    // Mirror to ARM7 WRAM
                    1 => address & 0x3FFF,            // First half
                    2 => (address & 0x3FFF) + 0x4000, // Second half
                    3 => address & 0x7FFF,            // Entire 32 KB
                    _ => return,
                } as usize;

                self.shared_wram[off] = byte
            }

            // IO register byte writes
            0x04000138 => self.rtc.write(byte as u16, true),

            0x040001A1 => self.cart.set_hi_auxspicnt(byte),

            // Cart command sequence
            0x040001A8..=0x040001AF => self
                .cart
                .receive_command(byte, (address - 0x040001A8) as usize),

            0x040001C2 => self.spi.write_spidata(byte),

            0x04000208 => self.int7_reg_ime = byte & 1,

            0x04000300 => self.postflg7 = byte & 1,

            0x04000301 => {
                match byte {
                    0x80 => {
                        // ARM7 halt request
                        self.arm7.halt()
                    }
                    _ => {
                        #[cfg(feature = "tracing")]
                        tracing::error!("Unrecognized HALTCNT value {:02X} for ARM7", byte);
                        // original code exits, but emulator should not crash
                    }
                }
            }

            0x04000500 => self.spu.set_soundcnt_lo(byte),

            0x04000501 => self.spu.set_soundcnt_hi(byte),

            0x04000508 => self.spu.set_sndcap0(byte),

            0x04000509 => self.spu.set_sndcap1(byte),

            // SPU channel writes
            0x04000400..0x04000500 => self.spu.write_channel_byte(address, byte),

            // GPU VRAM (ARM7 writes allowed)
            0x06000000..0x07000000 => self.gpu.write_arm7_u8(address, byte),

            // Ignore BIOS writes
            0x00000000..0x00004000 => {}

            // Default
            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!(
                    "(7) Unrecognized byte write {:02X} -> {:08X}",
                    byte,
                    address
                );
            }
        }
    }
}
