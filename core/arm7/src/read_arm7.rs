// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::emulator::Emulator;
use crate::interrupts::Interrupt;
use mem_const::*;

impl Emulator {
    /// ARM7 read 32-bit word
    pub fn arm7_read_word(&mut self, address: u32) -> u32 {
        // Main RAM
        if (MAIN_RAM_START..SHARED_WRAM_START).contains(&address) {
            let off = (address & MAIN_RAM_MASK) as usize;
            return u32::from_le_bytes(self.main_ram[off..off + 4].try_into().unwrap());
        }

        if (SHARED_WRAM_START..ARM7_WRAM_START).contains(&address) {
            let off = match self.wramcnt {
                0 => address & ARM7_WRAM_MASK,    // Mirror to ARM7 WRAM
                1 => address & 0x3FFF,            // First half
                2 => (address & 0x3FFF) + 0x4000, // Second half
                3 => address & 0x7FFF,            // Entire 32 KB
                _ => return 0,                    // TODO: Log trancing error
            } as usize;
            return u32::from_le_bytes(self.arm7_wram[off..off + 4].try_into().unwrap());
        }

        match address {
            0x040000DC => {
                return (self.dma.read_len(7) | self.dma.read_cnt(7) << 16).into();
            }
            0x04000120 => {
                return 0;
            }
            0x04000180 => {
                return self.ipcsync_nds7.read().into();
            }
            0x040001A4 => {
                return self.cart.get_romctrl();
            }
            0x040001C0 => {
                return (self.spi.get_spicnt() | (self.spi.read_spidata() << 16) as u16).into();
            }
            0x04000208 => {
                return self.int7_reg_ime as u32;
            }
            0x04000210 => {
                return self.int7_reg_ie;
            }
            0x04000214 => {
                return self.int7_reg_if;
            }
            0x04000218 => {
                return 0;
            }
            0x04000500 => {
                return 0;
            }
            0x04004008 => {
                return 0;
            }
            0x04100000 => {
                let word: u32 = self.fifo9.read_queue();

                if self.fifo9.request_empty_irq {
                    self.request_interrupt9(Interrupt::IPC_FIFO_EMPTY);
                    self.fifo9.request_empty_irq = false;
                }

                return word;
                // uint32_t word = fifo9.read_queue();
                // if (fifo9.request_empty_IRQ) {
                // request_interrupt9(INTERRUPT::IPC_FIFO_EMPTY);
                // fifo9.request_empty_IRQ = false;
                // }
                // return word;
            }
            0x04100010 => {
                // return cart.get_output();
            }
            _ => {}
        }

        if address >= GBA_ROM_START {
            return 0xFFFF_FFFF;
        }

        println!("\n(9) Unrecognized word read from ${:08X}", address);
        0
    }
}
