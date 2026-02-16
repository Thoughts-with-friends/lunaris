// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! dma.hpp
//!
//! Direct Memory Access (DMA) controller for Nintendo DS
//! Manages high-speed memory transfers between memory regions
use crate::emulator::Emulator;
use crate::interrupts::Interrupt;

impl Emulator {
    /// Handle scheduler event
    pub fn dma_handle_event(&mut self) {
        #[cfg(feature = "tracing")]
        tracing::trace!("Emulator::dma_handle_event has called.");

        self.dma_event.processing = false;
        let event_id = self.dma_event.id;

        loop {
            let (is_arm9, internal_len, length, irq_after_transfer, interrupt7, interrupt9) = {
                let active_dma = &self.dma.dmas[event_id as usize];
                let is_arm9 = active_dma.is_arm9;
                let internal_len = active_dma.internal_len;
                let length = active_dma.length;
                let irq_after_transfer = active_dma.cnt.irq_after_transfer;
                let interrupt9 = (8 + active_dma.index) as usize;
                let interrupt7 = (4 + active_dma.index) as usize;

                (
                    is_arm9,
                    internal_len,
                    length,
                    irq_after_transfer,
                    interrupt7,
                    interrupt9,
                )
            };

            if internal_len > length {
                if irq_after_transfer {
                    if is_arm9 {
                        if let Some(interrupt) = Interrupt::from_usize(interrupt9) {
                            self.request_interrupt9(interrupt);
                        } else {
                            #[cfg(feature = "tracing")]
                            tracing::error!("Out of index {interrupt9}");
                        }
                    } else if let Some(interrupt) = Interrupt::from_usize(interrupt7) {
                        self.request_interrupt7(interrupt);
                    } else {
                        #[cfg(feature = "tracing")]
                        tracing::error!("Out of index {interrupt7}");
                    }
                }

                let repeat = {
                    let active_dma = &mut self.dma.dmas[event_id as usize];
                    active_dma.internal_len = 0;
                    active_dma.cnt.repeat
                };
                if !repeat {
                    let active_dma = &mut self.dma.dmas[event_id as usize];
                    active_dma.cnt.enabled = false;
                    self.dma.active_dmas &= (!(1 << event_id)) as u8;
                } else {
                    //Reload dest
                    let (dest_control, timing) = {
                        let active_dma = &self.dma.dmas[event_id as usize];
                        (active_dma.cnt.dest_control, active_dma.cnt.timing)
                    };

                    if dest_control == 3 {
                        let active_dma = &mut self.dma.dmas[event_id as usize];
                        active_dma.internal_dest = active_dma.destination;
                    }
                    if timing != 0 {
                        self.dma.active_dmas &= (!(1 << event_id)) as u8;
                    } else {
                        let length = self.dma.dmas[event_id as usize].length;
                        self.add_dma_event(event_id, length as u64);
                    }
                }
                return;
            }

            // transfer 1 unit
            let (is_arm9, word_transfer, internal_source, internal_dest) = {
                let active_dma = &self.dma.dmas[event_id as usize];
                (
                    active_dma.is_arm9,
                    active_dma.cnt.word_transfer,
                    active_dma.internal_source,
                    active_dma.internal_dest,
                )
            };

            let (_value, offset) = if word_transfer {
                let value = if is_arm9 {
                    let v = self.arm9_read_word(internal_source);
                    self.arm9_write_word(internal_dest, v);
                    v
                } else {
                    let v = self.arm7_read_word(internal_source);
                    self.arm7_write_word(internal_dest, v);
                    v
                };
                (value, 4_u32)
            } else {
                let value = if is_arm9 {
                    let v = self.arm9_read_halfword(internal_source) as u32;
                    self.arm9_write_halfword(internal_dest, v as u16);
                    v
                } else {
                    let v = self.arm7_read_halfword(internal_source) as u32;
                    self.arm7_write_halfword(internal_dest, v as u16);
                    v
                };
                (value, 2_u32)
            };

            // destination control
            {
                let active_dma = &mut self.dma.dmas[event_id as usize];
                match active_dma.cnt.dest_control {
                    0 | 3 => {
                        active_dma.internal_dest = active_dma.internal_dest.wrapping_add(offset)
                    }
                    1 => active_dma.internal_dest = active_dma.internal_dest.wrapping_sub(offset),
                    2 => {}
                    v => {
                        #[cfg(feature = "tracing")]
                        tracing::error!("Unrecognized DMA dest control {v}");
                    }
                }
            }

            // source control
            {
                let active_dma = &mut self.dma.dmas[event_id as usize];
                match active_dma.cnt.source_control {
                    0 => {
                        active_dma.internal_source = active_dma.internal_source.wrapping_add(offset)
                    }
                    1 => {
                        active_dma.internal_source = active_dma.internal_source.wrapping_sub(offset)
                    }
                    2 => {}
                    v => {
                        #[cfg(feature = "tracing")]
                        tracing::error!("Unrecognized DMA source control {v}");
                    }
                }
            }

            // special ARM9 timing 7 behavior
            let special_stop = {
                let active_dma = &self.dma.dmas[event_id as usize];
                active_dma.is_arm9 && active_dma.cnt.timing == 7 && active_dma.internal_len >= 112
            };

            if special_stop {
                let active_dma = &mut self.dma.dmas[event_id as usize];
                active_dma.internal_len = 0;
                active_dma.length -= 112;

                self.dma.active_dmas &= (!(1 << event_id)) as u8;
                return;
            }
        }
    }

    /// Placeholder for DMA event processing.
    /// Currently does nothing.
    pub fn dma_event(&mut self, _index: u32) {
        // let active_dma = &self.dma.dmas[index as usize];
        // while active_dma.internal_len > active_dma.length {
        //     std::hint::spin_loop();
        // }
        panic!("C++ code is not called.")
    }

    /// Write control register to DMA channel
    pub fn dma_write_cnt(&mut self, index: usize, cnt: u16) {
        let dma = &mut self.dma.dmas[index];
        let old_enabled = dma.cnt.enabled;

        dma.cnt.set(cnt);

        if !old_enabled && (cnt & (1 << 15) != 0) {
            dma.internal_source = dma.source;
            dma.internal_dest = dma.destination;
            dma.internal_len = 0;

            match dma.cnt.timing {
                0 => {
                    self.dma.active_dmas |= 1 << index;
                    self.add_dma_event(index as i32, 0);
                }
                7 => self.check_gxfifo_dma(),
                _ => {}
            }
        }
    }

    /// Write length and control register as single word
    pub fn dma_write_len_cnt(&mut self, index: usize, word: u32) {
        if index >= self.dma.dmas.len() {
            return;
        }

        self.dma.write_len(index, (word & 0xFFFF) as u16);
        self.dma_write_cnt(index, (word >> 16) as u16);
    }

    /// Request HBLANK-triggered DMA transfers
    pub fn hblank_request(&mut self) {
        for i in 0..4 {
            let dma = &self.dma.dmas[i];
            if dma.cnt.enabled && dma.cnt.timing == 2 && self.dma.active_dmas == 0 {
                self.dma.active_dmas |= 1 << i;
                self.add_dma_event(i as i32, 0);
                break;
            }
        }
    }

    /// Request game cartridge DMA transfer
    pub fn gamecart_request(&mut self) {
        if self.arm7_has_cart_rights() {
            // NDS7 DMAs
            for i in 4..8 {
                let dma = &self.dma.dmas[i];
                if dma.cnt.enabled && dma.cnt.timing == 4 {
                    self.dma.active_dmas |= 1 << i;
                    self.add_dma_event(i as i32, 0);
                    break;
                }
            }
        } else {
            for i in 0..4 {
                let dma = &self.dma.dmas[i];
                if dma.cnt.enabled && dma.cnt.timing == 5 {
                    self.dma.active_dmas |= 1 << i;
                    self.add_dma_event(i as i32, 0);
                    break;
                }
            }
        }
    }

    /// Request GXFIFO DMA transfer
    pub fn gxfifo_request(&mut self) {
        for i in 0..4 {
            let dma = &self.dma.dmas[i];
            if dma.cnt.enabled && dma.cnt.timing == 7 {
                self.dma.active_dmas |= 1 << i;
                self.add_dma_event(i as i32, 0);
                break;
            }
        }
    }
}
