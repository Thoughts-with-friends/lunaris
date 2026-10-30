// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! dma.hpp
//!
//! Direct Memory Access (DMA) controller for Nintendo DS
//! Manages high-speed memory transfers between memory regions
use ds_interrupt::Interrupt;

use crate::emulator::Emulator;

impl Emulator {
    /// Runs the DMA channel whose scheduler event came due, transferring
    /// all of its units at once.
    ///
    /// ```text
    /// from: run() step 2f (dma_event.activation_time reached); the event
    ///       was added by dma_write_cnt (immediate), hblank_request,
    ///       gamecart_request, gxfifo_request or a repeating DMA
    ///   |
    ///   v
    /// dma_handle_event()
    ///   loop per unit:
    ///     1. internal_len += 1; all units done?
    ///          yes: IRQ (DMA0-3 of the owning CPU) if enabled
    ///               repeat off -> disable channel, clear active bit
    ///               repeat on  -> reload dest (mode 3); immediate timing
    ///                             re-schedules itself, others wait for
    ///                             their trigger again
    ///               return
    ///     2. copy one halfword/word with the owning CPU's bus
    ///        (arm9_/arm7_read_* -> arm9_/arm7_write_*, so I/O side
    ///        effects such as GXFIFO writes happen)
    ///     3. step dest / source by +size, -size or fixed
    ///     4. ARM9 GXFIFO timing (7): stop after 112 units, keep the rest
    ///        for the next FIFO request
    ///   |
    ///   v
    /// to:   run(); execute() resumes the CPUs once active_dmas is 0
    /// ```
    pub fn dma_handle_event(&mut self) {
        #[cfg(feature = "log")]
        log::debug!("Emulator::dma_handle_event has called.");

        self.dma_event.processing = false;
        let event_id = self.dma_event.id;

        loop {
            // Advance the unit counter before checking for completion, matching
            // the C++ `active_DMA->internal_len++` at the top of the loop. This
            // increment was lost in a prior refactor, which made the `internal_len
            // > length` check below never true and left the transfer looping
            // forever while wrapping source/dest addresses across all of memory.
            {
                let active_dma = &mut self.dma.dmas[event_id as usize];
                active_dma.internal_len += 1;
            }

            let (is_arm9, internal_len, length, irq_after_transfer, interrupt7, interrupt9) = {
                let active_dma = &self.dma.dmas[event_id as usize];
                let is_arm9 = active_dma.is_arm9;
                let internal_len = active_dma.internal_len;
                let length = active_dma.length;
                let irq_after_transfer = active_dma.cnt.irq_after_transfer;
                let interrupt9 = (8 + active_dma.index) as usize;
                // ARM7 channels are 4-7 but raise DMA0-3 (IF bits 8-11).
                let interrupt7 = (8 + (active_dma.index & 3)) as usize;

                (is_arm9, internal_len, length, irq_after_transfer, interrupt7, interrupt9)
            };

            if internal_len > length {
                #[cfg(feature = "log")]
                log::debug!(
                    target: "dma",
                    "dma{event_id} done len={length} src={:08X} dst={:08X} irq={irq_after_transfer} gxfifo={} pipe={} IF9={:08X}",
                    self.dma.dmas[event_id as usize].internal_source,
                    self.dma.dmas[event_id as usize].internal_dest,
                    self.gpu.engine_3d.gxfifo.len(),
                    self.gpu.engine_3d.gxpipe.len(),
                    self.int9_reg.irq_flags
                );
                if irq_after_transfer {
                    if is_arm9 {
                        if let Some(interrupt) = Interrupt::from_usize(interrupt9) {
                            self.request_interrupt9(interrupt);
                        } else {
                            #[cfg(feature = "log")]
                            log::error!("Out of index {interrupt9}");
                        }
                    } else if let Some(interrupt) = Interrupt::from_usize(interrupt7) {
                        self.request_interrupt7(interrupt);
                    } else {
                        #[cfg(feature = "log")]
                        log::error!("Out of index {interrupt7}");
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
                    // #[cfg(feature = "log")]
                    // log::debug!(
                    //     "DMA Access [ARM9_READ] Addr: 0x{internal_dest:08X} Word: 0x{v:08X}",
                    // );
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
                        active_dma.internal_dest = active_dma.internal_dest.wrapping_add(offset);
                    }
                    1 => active_dma.internal_dest = active_dma.internal_dest.wrapping_sub(offset),
                    2 => {}
                    _v => {
                        #[cfg(feature = "log")]
                        log::error!("Unrecognized DMA dest control {_v}");
                    }
                }
            }

            // source control
            {
                let active_dma = &mut self.dma.dmas[event_id as usize];
                match active_dma.cnt.source_control {
                    0 => {
                        active_dma.internal_source =
                            active_dma.internal_source.wrapping_add(offset);
                    }
                    1 => {
                        active_dma.internal_source =
                            active_dma.internal_source.wrapping_sub(offset);
                    }
                    2 => {}
                    _v => {
                        #[cfg(feature = "log")]
                        log::error!("Unrecognized DMA source control {_v}");
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
    /// # Panics
    pub fn dma_event(&mut self, _index: u32) {
        // let active_dma = &self.dma.dmas[index as usize];
        // while active_dma.internal_len > active_dma.length {
        //     std::hint::spin_loop();
        // }
        panic!("C++ code is not called.")
    }

    /// DMAxCNT high halfword write: updates the control bits and starts
    /// the channel on a 0 -> 1 edge of the enable bit.
    ///
    /// ```text
    /// from: arm9_/arm7_write_halfword(DMAxCNT_H) / dma_write_len_cnt()
    ///   |
    ///   v
    /// dma_write_cnt(index, cnt)
    ///   1. cnt.set(cnt)                 (start timing width per CPU)
    ///   2. newly enabled?  latch source/dest, internal_len = 0
    ///        timing 0 (immediate) -> mark active, add_dma_event(now)
    ///        timing 7 (GXFIFO)    -> check_gxfifo_dma()
    ///        other timings        -> wait for hblank_request /
    ///                                gamecart_request / ...
    ///   |
    ///   v
    /// to:   dma_handle_event() via the scheduler
    /// ```
    pub fn dma_write_cnt(&mut self, index: usize, cnt: u16) {
        #[cfg(feature = "log")]
        log::debug!("call dma_write_cnt: index: {index}, cnt: {cnt}");

        let dma = &mut self.dma.dmas[index];
        let old_enabled = dma.cnt.enabled;
        let is_arm9 = dma.is_arm9;

        dma.cnt.set(cnt, is_arm9);

        #[cfg(feature = "log")]
        log::debug!(
            target: "dma",
            "dma{index} cnt={cnt:04X} (was {}) src={:08X} dst={:08X} len={}",
            if old_enabled { "on" } else { "off" },
            dma.source,
            dma.destination,
            dma.length
        );
        // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
        if !old_enabled && (cnt & (1 << 15) != 0) {
            // DMAxCNT_H bit 15: enable (0 -> 1 edge)
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

    /// 32-bit DMAxCNT write: low half = length (`dma.write_len`), high
    /// half = control (`dma_write_cnt`).
    pub fn dma_write_len_cnt(&mut self, index: usize, word: u32) {
        if index >= self.dma.dmas.len() {
            return;
        }

        // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
        self.dma.write_len(index, (word & 0xFFFF) as u16); // DMAxCNT_L word count (lower halfword)
        self.dma_write_cnt(index, (word >> 16) as u16);
    }

    /// Starts the first enabled ARM9 HBlank DMA (timing 2).
    ///
    /// ```text
    /// from: gpu_handle_event() HBlank start (visible lines) ->
    ///       hblank_dma_request()
    /// to:   add_dma_event(now) -> dma_handle_event()
    /// ```
    pub fn hblank_request(&mut self) {
        for i in 0..4 {
            // for ARM9
            let dma = &self.dma.dmas[i];
            if dma.cnt.enabled && dma.cnt.timing == 2 && self.dma.active_dmas == 0 {
                self.dma.active_dmas |= 1 << i;
                self.add_dma_event(i as i32, 0);
                break;
            }
        }
    }

    /// Starts the cartridge DMA of the CPU that owns the slot (EXMEMCNT
    /// bit 11): ARM7 channels with timing 4 or ARM9 channels with timing 5.
    ///
    /// ```text
    /// from: cartridge_run() when a ROM data word is ready ->
    ///       gamecart_dma_request()
    /// to:   add_dma_event(now) -> dma_handle_event() reads 0x04100010
    /// ```
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

    /// Starts the first enabled ARM9 GXFIFO DMA (timing 7).
    ///
    /// ```text
    /// from: check_fifo_dma() / request_fifo_dma() / check_gxfifo_dma()
    ///       (GXFIFO less than half full) -> gxfifo_dma_request()
    /// to:   add_dma_event(now) -> dma_handle_event() (112 words max)
    /// ```
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
