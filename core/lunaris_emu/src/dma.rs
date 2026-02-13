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
        DmaCnt {
            dest_control: 0,
            source_control: 0,
            repeat: false,
            word_transfer: false,
            timing: 0,
            irq_after_transfer: false,
            enabled: false,
        }
    }

    /// Get register value as 16-bit halfword
    pub fn get(&self) -> u16 {
        let mut value = 0u16;
        value |= ((self.dest_control & 0x3) as u16) << 5;
        value |= ((self.source_control & 0x3) as u16) << 7;

        if self.repeat {
            value |= 1 << 9;
        }
        if self.word_transfer {
            value |= 1 << 10;
        }
        value |= ((self.timing & 0x3) as u16) << 11;
        if self.irq_after_transfer {
            value |= 1 << 14;
        }
        if self.enabled {
            value |= 1 << 15;
        }
        value
    }

    /// Set register value from 16-bit halfword
    pub fn set(&mut self, value: u16) {
        self.dest_control = ((value >> 5) & 0x3) as u32;
        self.source_control = ((value >> 7) & 0x3) as u32;
        self.repeat = (value & (1 << 9)) != 0;
        self.word_transfer = (value & (1 << 10)) != 0;
        self.timing = ((value >> 11) & 0x3) as u32;
        self.irq_after_transfer = (value & (1 << 14)) != 0;
        self.enabled = (value & (1 << 15)) != 0;
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
        Dma {
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

        NDSDma {
            dmas,
            // active_dma7: None,
            // active_dma9: None,
            active_dmas: 0,
        }
    }

    /// Power on DMA controller
    pub fn power_on(&mut self) {
        self.active_dmas = 0;

        for i in 0..8 {
            self.dmas[i].is_arm9 = i < 4;
            self.dmas[i].cnt.set(0);
        }
    }

    // moved emulator/dma.rs:
    // pub fn dma_event(&mut self, index: u32);

    /// Update and process DMA transfer
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
            true => self.dmas[index].cnt.get(),
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
