// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! dma.hpp
//!
//! Direct Memory Access (DMA) controller for Nintendo DS
//! Manages high-speed memory transfers between memory regions
//!
//! Start triggers follow melonDS (`NDS::CheckDMAs` / `StopDMAs`):
//!
//! ```text
//!  DMAxCNT write, enable 0 -> 1 ---- immediate --------------> start_dma()
//!                               `--- cart / GXFIFO: start now if the
//!                                    device is already requesting
//!  gpu_handle_event  line 2..193 start ----- ARM9 3 (display start)
//!                    line 194         stop - ARM9 3
//!                    HBlank, line < 192 ---- ARM9 2
//!                    line 192 (VBlank) ----- ARM9 1, ARM7 1; stop ARM9 4
//!  display FIFO      every 8 pixels -------- ARM9 4 (main memory display)
//!  cartridge_run     word ready ------------ ARM9 5 / ARM7 2 (slot owner)
//!  GXFIFO            less than half full --- ARM9 7
//!        |
//!        v
//!  check_dmas(mode) -> start_dma(ch): latch the count, mark active,
//!                      add_dma_event(now) -> dma_handle_event()
//! ```
use ds_gpu::display::fifo::FIFO_STEP;
use ds_interrupt::Interrupt;

use crate::{
    dma::{
        MODE_IMMEDIATE, MODE7_CART, MODE7_IMMEDIATE, MODE7_VBLANK, MODE9_CART, MODE9_DISPLAY_START,
        MODE9_GXFIFO, MODE9_HBLANK, MODE9_MAIN_MEMORY_DISPLAY, MODE9_VBLANK,
    },
    emulator::Emulator,
};

/// Units a GXFIFO DMA moves per request (GBATEK "DS DMA Transfers").
const GXFIFO_BURST: u32 = 112;

impl Emulator {
    /// Runs every DMA channel that was started, lowest channel first,
    /// each moving all the units of its burst at once.
    ///
    /// ```text
    /// from: run() step 2f (dma_event.activation_time reached); the event
    ///       was added by start_dma()
    ///   |
    ///   v
    /// dma_handle_event() -> run_dma(ch) for each active channel
    ///   1. burst = all units left; GXFIFO: at most 112; cartridge: 1
    ///   2. copy each unit with the owning CPU's bus
    ///      (arm9_/arm7_read_* -> arm9_/arm7_write_*, so I/O side
    ///      effects such as GXFIFO writes happen)
    ///   3. units left -> wait for the next request (in_progress)
    ///      done       -> repeat off: disable; IRQ (DMA0-3) if enabled
    ///   |
    ///   v
    /// to:   run(); execute() resumes the CPUs once active_dmas is 0
    /// ```
    ///
    /// Channels started while this runs (a GXFIFO write requesting more
    /// data, ...) are not in `pending` and run on their own event.
    pub fn dma_handle_event(&mut self) {
        self.dma_event.processing = false;
        let pending = self.dma.active_dmas;
        for index in 0..8 {
            if pending & (1 << index) != 0 {
                self.run_dma(index);
            }
        }
    }

    /// Moves one burst of channel `index` (see [`Self::dma_handle_event`]).
    fn run_dma(&mut self, index: usize) {
        let dma = self.dma.dmas[index];
        let mode = dma.start_mode();
        let units = match mode {
            MODE9_GXFIFO => dma.length.min(GXFIFO_BURST),
            // One word per cartridge data request (ROMCTRL bit 23).
            MODE9_CART | MODE7_CART => dma.length.min(1),
            _ => dma.length,
        };

        let size: u32 = if dma.cnt.word_transfer { 4 } else { 2 };
        // Address control: 0 increment, 1 decrement, 2 fixed, 3 increment
        // (+ reload of the destination on repeat).
        let step = |control: u32| match control {
            1 => size.wrapping_neg(),
            2 => 0,
            _ => size,
        };
        let (src_step, dst_step) = (step(dma.cnt.source_control), step(dma.cnt.dest_control));
        let (mut src, mut dst) = (dma.internal_source, dma.internal_dest);

        for _ in 0..units {
            match (dma.is_arm9, dma.cnt.word_transfer) {
                (true, true) => {
                    let v = self.arm9_read_word(src);
                    self.arm9_write_word(dst, v);
                }
                (true, false) => {
                    let v = self.arm9_read_halfword(src);
                    self.arm9_write_halfword(dst, v);
                }
                (false, true) => {
                    let v = self.arm7_read_word(src);
                    self.arm7_write_word(dst, v);
                }
                (false, false) => {
                    let v = self.arm7_read_halfword(src);
                    self.arm7_write_halfword(dst, v);
                }
            }
            src = src.wrapping_add(src_step);
            dst = dst.wrapping_add(dst_step);
        }

        self.dma.active_dmas &= !(1 << index);
        let d = &mut self.dma.dmas[index];
        d.internal_source = src;
        d.internal_dest = dst;
        d.length -= units;

        #[cfg(feature = "log")]
        log::debug!(
            target: "dma",
            "dma{index} mode={mode:02X} moved {units} left={} src={src:08X} dst={dst:08X}",
            d.length
        );

        if d.length > 0 {
            return; // the rest goes on the next request
        }
        d.in_progress = false;
        // An immediate DMA with repeat set stays enabled but never
        // restarts (melonDS `DMA::Run`).
        if !d.cnt.repeat {
            d.cnt.enabled = false;
        }
        if d.cnt.irq_after_transfer {
            // ARM7 channels are 4-7 but raise DMA0-3 (IF bits 8-11).
            if let Some(irq) = Interrupt::from_usize(8 + (index & 3)) {
                if dma.is_arm9 {
                    self.request_interrupt9(irq);
                } else {
                    self.request_interrupt7(irq);
                }
            }
        }
    }

    /// Starts channel `index` unless it is already running. A new
    /// transfer latches the word count and, with address control 3,
    /// reloads the addresses; one that is `in_progress` continues.
    fn start_dma(&mut self, index: usize) {
        if self.dma.active_dmas & (1 << index) != 0 {
            return;
        }
        let d = &mut self.dma.dmas[index];
        if !d.in_progress {
            d.length = d.start_count();
            d.in_progress = true;
            if d.cnt.source_control == 3 {
                d.internal_source = d.source;
            }
            if d.cnt.dest_control == 3 {
                d.internal_dest = d.destination;
            }
        }
        self.dma.active_dmas |= 1 << index;
        self.add_dma_event(0);
    }

    /// Starts every enabled channel of the CPU in `mode` (melonDS start
    /// mode: ARM9 timing, or `0x10 |` ARM7 timing).
    pub fn check_dmas(&mut self, mode: u32) {
        let base = if mode & 0x10 != 0 { 4 } else { 0 };
        for index in base..base + 4 {
            let dma = &self.dma.dmas[index];
            if dma.cnt.enabled && dma.start_mode() == mode {
                self.start_dma(index);
            }
        }
    }

    /// Disables every channel of the CPU in `mode` (display start DMAs at
    /// line 194, main memory display DMAs at VBlank).
    pub fn stop_dmas(&mut self, mode: u32) {
        let base = if mode & 0x10 != 0 { 4 } else { 0 };
        for dma in &mut self.dma.dmas[base..base + 4] {
            if dma.start_mode() == mode {
                dma.cnt.enabled = false;
                dma.in_progress = false;
            }
        }
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
    ///   2. newly enabled?  latch source/dest
    ///        immediate          -> start_dma()
    ///        cartridge          -> start now if a data word is waiting
    ///        GXFIFO             -> check_gxfifo_dma()
    ///        other timings      -> wait for check_dmas()
    ///   |
    ///   v
    /// to:   dma_handle_event() via the scheduler
    /// ```
    pub fn dma_write_cnt(&mut self, index: usize, cnt: u16) {
        let dma = &mut self.dma.dmas[index];
        let old_enabled = dma.cnt.enabled;
        let is_arm9 = dma.is_arm9;

        dma.cnt.set(cnt, is_arm9);

        #[cfg(feature = "log")]
        log::debug!(
            target: "dma",
            "dma{index} cnt={cnt:04X} (was {}) src={:08X} dst={:08X} count={}",
            if old_enabled { "on" } else { "off" },
            dma.source,
            dma.destination,
            dma.start_count()
        );
        // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
        if old_enabled || !dma.cnt.enabled {
            if !dma.cnt.enabled {
                dma.in_progress = false;
            }
            return;
        }
        // DMAxCNT_H bit 15: enable (0 -> 1 edge)
        dma.internal_source = dma.source;
        dma.internal_dest = dma.destination;
        dma.in_progress = false;

        match dma.start_mode() {
            MODE_IMMEDIATE | MODE7_IMMEDIATE => self.start_dma(index),
            MODE9_CART | MODE7_CART => {
                if self.cart.romctrl.word_ready {
                    self.gamecart_request();
                }
            }
            MODE9_GXFIFO => self.check_gxfifo_dma(),
            _ => {}
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

    /// Starts the ARM9 HBlank DMAs (timing 2).
    ///
    /// ```text
    /// from: gpu_handle_event() HBlank start (visible lines) ->
    ///       hblank_dma_request()
    /// ```
    pub fn hblank_request(&mut self) {
        self.check_dmas(MODE9_HBLANK);
    }

    /// VBlank start (line 192): ARM9 and ARM7 VBlank DMAs start, main
    /// memory display DMAs stop.
    pub fn vblank_dma_request(&mut self) {
        self.stop_dmas(MODE9_MAIN_MEMORY_DISPLAY);
        self.check_dmas(MODE9_VBLANK);
        self.check_dmas(MODE7_VBLANK);
    }

    /// Feeds the main memory display FIFO for the line about to be drawn:
    /// every 8 pixels the ARM9 "main memory display" DMAs (timing 4) run
    /// right away, then 8 pixels leave the FIFO (melonDS
    /// `GPU::DisplayFIFO`). Skipped when nothing uses the FIFO.
    pub fn fill_display_fifo_line(&mut self) {
        let has_dma = self.dma.dmas[..4]
            .iter()
            .any(|d| d.cnt.enabled && d.start_mode() == MODE9_MAIN_MEMORY_DISPLAY);
        if !has_dma && !self.gpu.uses_display_fifo() {
            return;
        }
        for x in (0..ds_mem_addr::PIXELS_PER_LINE).step_by(FIFO_STEP) {
            for index in 0..4 {
                let dma = &self.dma.dmas[index];
                if dma.cnt.enabled && dma.start_mode() == MODE9_MAIN_MEMORY_DISPLAY {
                    self.start_dma(index);
                    if self.dma.active_dmas & (1 << index) != 0 {
                        self.run_dma(index);
                    }
                }
            }
            self.gpu.display_fifo.sample(x, FIFO_STEP);
        }
    }

    /// Start of a line: ARM9 "start of display" DMAs run on lines 2..193
    /// and are disabled at line 194 (melonDS `GPU::StartScanline`).
    pub fn display_start_dma_request(&mut self, line: u16) {
        match line {
            2..194 => self.check_dmas(MODE9_DISPLAY_START),
            194 => self.stop_dmas(MODE9_DISPLAY_START),
            _ => {}
        }
    }

    /// Starts the cartridge DMAs of the CPU that owns the slot (EXMEMCNT
    /// bit 11): ARM7 timing 2 or ARM9 timing 5.
    ///
    /// ```text
    /// from: cartridge_run() when a ROM data word is ready ->
    ///       gamecart_dma_request()
    /// to:   start_dma() -> dma_handle_event() reads 0x04100010
    /// ```
    pub fn gamecart_request(&mut self) {
        if self.arm7_has_cart_rights() {
            self.check_dmas(MODE7_CART);
        } else {
            self.check_dmas(MODE9_CART);
        }
    }

    /// Starts the ARM9 GXFIFO DMAs (timing 7).
    ///
    /// ```text
    /// from: check_fifo_dma() / request_fifo_dma() / check_gxfifo_dma()
    ///       (GXFIFO less than half full) -> gxfifo_dma_request()
    /// to:   start_dma() -> dma_handle_event() (112 units max)
    /// ```
    pub fn gxfifo_request(&mut self) {
        self.check_dmas(MODE9_GXFIFO);
    }
}
