// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! dma.hpp
//!
//! Direct Memory Access (DMA) controller for Nintendo DS
//! Manages high-speed memory transfers between memory regions

/// DMA control register
#[derive(Debug, Clone, Copy)]
pub struct DmaCnt {
    /// Destination address control (0=increment, 1=decrement, 2=fixed, 3=reload)
    pub dest_control: u32,
    /// Source address control (0=increment, 1=decrement, 2=fixed)
    pub source_control: u32,
    /// Repeat the transfer when complete
    pub repeat: bool,
    /// Use 32-bit transfers instead of 16-bit
    pub word_transfer: bool,
    /// Start timing (0=immediate, 1=VBLANK, 2=HBLANK, 3=sync/special)
    pub timing: u32,
    /// Generate interrupt when transfer completes
    pub irq_after_transfer: bool,
    /// Enable this DMA channel
    pub enabled: bool,
}

impl DmaCnt {
    /// Create new DMA control register
    pub fn new() -> Self {
        Self {
            dest_control: 0,
            source_control: 0,
            repeat: false,
            word_transfer: false,
            timing: 0,
            irq_after_transfer: false,
            enabled: false,
        }
    }

    /// Get register value as 16-bit halfword.
    ///
    /// `is_arm9` selects the *DMA Start Timing* field width per GBATEK: 3
    /// bits (11-13) on ARM9, 2 bits (11-12) on ARM7.
    pub fn get(&self, is_arm9: bool) -> u16 {
        let timing_mask = if is_arm9 { 0x7 } else { 0x3 };

        let mut value = 0_u16;
        value |= ((self.dest_control & 0x3) as u16) << 5;
        value |= ((self.source_control & 0x3) as u16) << 7;

        if self.repeat {
            value |= 1 << 9;
        }
        if self.word_transfer {
            value |= 1 << 10;
        }
        value |= ((self.timing & timing_mask) as u16) << 11;
        if self.irq_after_transfer {
            value |= 1 << 14;
        }
        if self.enabled {
            value |= 1 << 15;
        }
        value
    }

    /// Set register value from 16-bit halfword.
    ///
    /// `is_arm9` selects the *DMA Start Timing* field width (see [`Self::get`]).
    /// Previously this always masked to 2 bits, so ARM9-only timings 4-7
    /// (Main-Memory-Display, DS-Cartridge, GBA-Cartridge, Geometry-Command-FIFO)
    /// could never be represented, silently preventing cartridge- and
    /// GXFIFO-triggered DMA from ever starting.
    pub fn set(&mut self, value: u16, is_arm9: bool) {
        let timing_mask = if is_arm9 { 0x7 } else { 0x3 };

        self.dest_control = ((value >> 5) & 0x3) as u32;
        self.source_control = ((value >> 7) & 0x3) as u32;
        self.repeat = (value & (1 << 9)) != 0;
        self.word_transfer = (value & (1 << 10)) != 0;
        self.timing = ((value >> 11) & timing_mask) as u32;
        self.irq_after_transfer = (value & (1 << 14)) != 0;
        self.enabled = (value & (1 << 15)) != 0;

        tracing::debug!(
            value = value,
            dest_control = self.dest_control,
            source_control = self.source_control,
            repeat = self.repeat,
            word_transfer = self.word_transfer,
            timing = self.timing,
            irq_after_transfer = self.irq_after_transfer,
            enabled = self.enabled,
            "DMA control register updated"
        );
    }
}

impl Default for DmaCnt {
    fn default() -> Self {
        Self::new()
    }
}

/// Individual DMA channel state
#[derive(Debug, Clone, Copy)]
pub struct Dma {
    /// Source address (user-controlled)
    pub source: u32,
    /// Source address (internal working copy)
    pub internal_source: u32,

    /// Destination address (user-controlled)
    pub destination: u32,
    /// Destination address (internal working copy)
    pub internal_dest: u32,

    /// Transfer length in units (user-controlled)
    pub length: u32,
    /// Transfer length (internal working copy)
    pub internal_len: u32,

    /// DMA control register
    pub cnt: DmaCnt,

    /// Channel index (0-7)
    pub index: u32,

    /// Is this ARM9 DMA (vs ARM7)
    pub is_arm9: bool,
}

impl Dma {
    /// Create new DMA channel
    pub fn new(index: u32, is_arm9: bool) -> Self {
        Self {
            source: 0,
            internal_source: 0,
            destination: 0,
            internal_dest: 0,
            length: 0,
            internal_len: 0,
            cnt: DmaCnt::new(),
            index,
            is_arm9,
        }
    }
}

impl Default for Dma {
    fn default() -> Self {
        Self::new(0, false)
    }
}

/// DMA Controller
/// Manages up to 8 DMA channels (4 ARM9, 4 ARM7) for fast memory transfers
#[derive(Debug, Default)]
pub struct NDSDma {
    /// DMA channels (0-7: 0-3 are ARM9, 4-7 are ARM7)
    pub dmas: [Dma; 8],

    /// Currently active ARM7 DMA index (unused)
    // active_dma7: Option<usize>,
    /// Currently active ARM9 DMA index (unused)
    // active_dma9: Option<usize>,

    /// Bitmask of which DMA channels are active
    pub active_dmas: u8,
}

impl NDSDma {
    /// Create new DMA controller
    pub fn new() -> Self {
        let mut dmas = [Dma::new(0, false); 8];

        // Initialize channels
        for (index, dma) in dmas.iter_mut().enumerate() {
            dma.is_arm9 = index < 4;
            dma.index = index as u32;
        }

        Self {
            dmas,
            // active_dma7: None,
            // active_dma9: None,
            active_dmas: 0,
        }
    }

    /// Power on DMA controller
    pub fn power_on(&mut self) {
        self.active_dmas = 0;

        for (i, dma) in self.dmas.iter_mut().enumerate() {
            dma.is_arm9 = i < 4;
            dma.cnt.set(0, dma.is_arm9);
        }
    }

    // moved emulator/dma.rs:
    // pub fn dma_event(&mut self, index: u32);

    /// Update and process DMA transfer
    #[expect(clippy::needless_pass_by_ref_mut)]
    pub fn update_dma(&mut self) {
        unimplemented!("C++ code is empty.")
    }

    // moved emulator/dma.rs:
    // pub fn handle_event(&mut self, _event: &SchedulerEvent);

    /// Check if any DMA channel is active
    pub fn is_active(&self) -> bool {
        self.active_dmas != 0
    }

    /// Read source address of DMA channel
    pub fn read_source(&self, index: usize) -> u32 {
        match index < 8 {
            true => self.dmas[index].source,
            false => 0,
        }
    }

    /// Read transfer length of DMA channel
    pub fn read_len(&self, index: usize) -> u16 {
        match index < 8 {
            true => (self.dmas[index].length & 0xFFFF) as u16,
            false => 0,
        }
    }

    /// Read control register of DMA channel
    pub fn read_cnt(&self, index: usize) -> u16 {
        match index < 8 {
            true => self.dmas[index].cnt.get(self.dmas[index].is_arm9),
            false => 0,
        }
    }

    /// Write source address to DMA channel
    pub fn write_source(&mut self, index: usize, source: u32) {
        if index < 8 {
            self.dmas[index].source = source;
        }
    }

    /// Write destination address to DMA channel
    pub fn write_dest(&mut self, index: usize, dest: u32) {
        if index < 8 {
            self.dmas[index].destination = dest;
        }
    }

    /// Write the low halfword of a DMA channel's source address (`DMAxSAD` bits 0-15).
    pub fn write_source_lo(&mut self, index: usize, lo: u16) {
        if index < 8 {
            let dma = &mut self.dmas[index];
            dma.source = (dma.source & 0xFFFF_0000) | u32::from(lo);
        }
    }

    /// Write the high halfword of a DMA channel's source address (`DMAxSAD` bits 16-31).
    pub fn write_source_hi(&mut self, index: usize, hi: u16) {
        if index < 8 {
            let dma = &mut self.dmas[index];
            dma.source = (dma.source & 0x0000_FFFF) | (u32::from(hi) << 16);
        }
    }

    /// Write the low halfword of a DMA channel's destination address (`DMAxDAD` bits 0-15).
    pub fn write_dest_lo(&mut self, index: usize, lo: u16) {
        if index < 8 {
            let dma = &mut self.dmas[index];
            dma.destination = (dma.destination & 0xFFFF_0000) | u32::from(lo);
        }
    }

    /// Write the high halfword of a DMA channel's destination address (`DMAxDAD` bits 16-31).
    pub fn write_dest_hi(&mut self, index: usize, hi: u16) {
        if index < 8 {
            let dma = &mut self.dmas[index];
            dma.destination = (dma.destination & 0x0000_FFFF) | (u32::from(hi) << 16);
        }
    }

    /// Write transfer length to DMA channel
    pub fn write_len(&mut self, index: usize, len: u16) {
        let dma = &mut self.dmas[index];

        let is_ch7 = index == 7;
        let len32 = len as u32;

        dma.length = match (dma.is_arm9, len == 0, is_ch7) {
            (true, true, _) => 0x200000,
            (true, false, _) => len32 & 0x1FFFFF,
            (false, true, true) => 0x10000,
            (false, true, false) => 0x4000,
            (false, false, true) => len32 & 0xFFFF,
            (false, false, false) => 0x3FFF,
        };
    }

    // moved emulator/dma.rs:
    // pub fn write_cnt(&mut self, index: usize, cnt: u16);
    // pub fn write_len_cnt(&mut self, index: usize, word: u32);
    // pub fn hblank_request(&mut self);
    // pub fn gamecart_request(&mut self);
    // pub fn gfxfifo_request(&mut self);
}
