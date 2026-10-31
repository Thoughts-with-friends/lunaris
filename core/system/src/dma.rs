// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! dma.hpp
//!
//! Direct Memory Access (DMA) controller for Nintendo DS
//! Manages high-speed memory transfers between memory regions

/// DMA control register (DMAxCNT_H, plus the ARM9's word count bits 16-20).
///
/// ```text
///  bit   15  14  13 12 11  10   9   8 7   6 5   4..0
///  ARM9  en  irq [timing ]  32  rep  src   dst   count bits 16-20
///  ARM7  en  irq [timing]   32  rep  src   dst   -
///                 (12-13)
/// ```
///
/// ARM9 timings: 0 immediate, 1 VBlank, 2 HBlank, 3 start of display,
/// 4 main memory display, 5 DS cart, 6 GBA cart, 7 GXFIFO.
/// ARM7 timings: 0 immediate, 1 VBlank, 2 DS cart, 3 Wi-Fi / GBA cart.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct DmaCnt {
    /// Destination address control (0=increment, 1=decrement, 2=fixed, 3=reload)
    pub dest_control: u32,
    /// Source address control (0=increment, 1=decrement, 2=fixed)
    pub source_control: u32,
    /// Repeat the transfer when complete
    pub repeat: bool,
    /// Use 32-bit transfers instead of 16-bit
    pub word_transfer: bool,
    /// Start timing field as written (3 bits on ARM9, 2 bits on ARM7).
    pub timing: u32,
    /// Generate interrupt when transfer completes
    pub irq_after_transfer: bool,
    /// Enable this DMA channel
    pub enabled: bool,
    /// ARM9 only: word count bits 16-20 (DMAxCNT_H bits 0-4).
    pub count_hi: u32,
}

impl DmaCnt {
    /// Create new DMA control register
    pub const fn new() -> Self {
        Self {
            dest_control: 0,
            source_control: 0,
            repeat: false,
            word_transfer: false,
            timing: 0,
            irq_after_transfer: false,
            enabled: false,
            count_hi: 0,
        }
    }

    /// Get register value as 16-bit halfword.
    pub fn get(&self, is_arm9: bool) -> u16 {
        // DS DMA Transfers: https://problemkaputt.de/gbatek.htm#dsdmatransfers
        let mut value = 0_u16;
        value |= ((self.dest_control & 0x3) as u16) << 5; // dest address control (bits 5-6)
        value |= ((self.source_control & 0x3) as u16) << 7; // source address control (bits 7-8)
        if self.repeat {
            value |= 1 << 9; // DMAxCNT_H bit 9: repeat
        }
        if self.word_transfer {
            value |= 1 << 10; // DMAxCNT_H bit 10: transfer type (1=32-bit)
        }
        if is_arm9 {
            value |= ((self.timing & 0x7) as u16) << 11; // ARM9 start timing: bits 11-13
            value |= (self.count_hi & 0x1F) as u16; // ARM9 word count bits 16-20
        } else {
            value |= ((self.timing & 0x3) as u16) << 12; // ARM7 start timing: bits 12-13
        }
        if self.irq_after_transfer {
            value |= 1 << 14; // DMAxCNT_H bit 14: IRQ upon end of word count
        }
        if self.enabled {
            value |= 1 << 15; // DMAxCNT_H bit 15: DMA enable
        }
        value
    }

    /// Set register value from 16-bit halfword.
    ///
    /// The start timing sits in bits 11-13 on the ARM9 but only in bits
    /// 12-13 on the ARM7 (GBATEK "DS DMA Transfers"); reading the ARM7 one
    /// from bit 11 turned its VBlank DMAs into timing 2 and its cartridge
    /// DMAs into immediate ones.
    pub fn set(&mut self, value: u16, is_arm9: bool) {
        self.dest_control = ((value >> 5) & 0x3) as u32; // dest address control (bits 5-6)
        self.source_control = ((value >> 7) & 0x3) as u32; // source address control (bits 7-8)
        self.repeat = (value & (1 << 9)) != 0; // DMAxCNT_H bit 9: repeat
        self.word_transfer = (value & (1 << 10)) != 0; // DMAxCNT_H bit 10: transfer type (1=32-bit)
        if is_arm9 {
            self.timing = ((value >> 11) & 0x7) as u32; // ARM9 start timing: bits 11-13
            self.count_hi = (value & 0x1F) as u32; // ARM9 word count bits 16-20
        } else {
            self.timing = ((value >> 12) & 0x3) as u32; // ARM7 start timing: bits 12-13
        }
        self.irq_after_transfer = (value & (1 << 14)) != 0; // DMAxCNT_H bit 14: IRQ upon end of word count
        self.enabled = (value & (1 << 15)) != 0; // DMAxCNT_H bit 15: DMA enable
    }
}

impl Default for DmaCnt {
    fn default() -> Self {
        Self::new()
    }
}

/// Start modes in melonDS numbering ([`Dma::start_mode`]): the ARM9 timing,
/// or `0x10 |` the ARM7 timing.
pub const MODE_IMMEDIATE: u32 = 0x00;
pub const MODE9_VBLANK: u32 = 0x01;
pub const MODE9_HBLANK: u32 = 0x02;
pub const MODE9_DISPLAY_START: u32 = 0x03;
pub const MODE9_MAIN_MEMORY_DISPLAY: u32 = 0x04;
pub const MODE9_CART: u32 = 0x05;
pub const MODE9_GXFIFO: u32 = 0x07;
pub const MODE7_IMMEDIATE: u32 = 0x10;
pub const MODE7_VBLANK: u32 = 0x11;
pub const MODE7_CART: u32 = 0x12;

/// Individual DMA channel state
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct Dma {
    /// Source address (user-controlled)
    pub source: u32,
    /// Source address (internal working copy)
    pub internal_source: u32,

    /// Destination address (user-controlled)
    pub destination: u32,
    /// Destination address (internal working copy)
    pub internal_dest: u32,

    /// Word count register bits 0-15 (DMAxCNT_L) as written.
    pub word_count: u32,
    /// Units left in the current transfer (latched from the word count
    /// when a transfer starts).
    pub length: u32,
    /// A transfer was started and still has units left (a GXFIFO or
    /// cartridge DMA waiting for its next request).
    pub in_progress: bool,

    /// DMA control register
    pub cnt: DmaCnt,

    /// Channel index (0-7)
    pub index: u32,

    /// Is this ARM9 DMA (vs ARM7)
    pub is_arm9: bool,
}

impl Dma {
    /// Create new DMA channel
    pub const fn new(index: u32, is_arm9: bool) -> Self {
        Self {
            source: 0,
            internal_source: 0,
            destination: 0,
            internal_dest: 0,
            word_count: 0,
            length: 0,
            in_progress: false,
            cnt: DmaCnt::new(),
            index,
            is_arm9,
        }
    }

    /// Start mode in melonDS numbering (see the `MODE*` constants).
    pub const fn start_mode(&self) -> u32 {
        if self.is_arm9 { self.cnt.timing } else { 0x10 | self.cnt.timing }
    }

    /// Units of one whole transfer: the word count, 0 meaning the maximum
    /// (ARM9 200000h, ARM7 DMA3 10000h, ARM7 DMA0-2 4000h).
    pub const fn start_count(&self) -> u32 {
        let mask = match (self.is_arm9, self.index & 3) {
            (true, _) => 0x1F_FFFF,
            (false, 3) => 0xFFFF,
            (false, _) => 0x3FFF,
        };
        let count = ((self.cnt.count_hi << 16) | self.word_count) & mask;
        if count == 0 { mask + 1 } else { count }
    }
}

impl Default for Dma {
    fn default() -> Self {
        Self::new(0, false)
    }
}

/// DMA Controller
/// Manages up to 8 DMA channels (4 ARM9, 4 ARM7) for fast memory transfers
#[derive(serde::Serialize, serde::Deserialize, Debug)]
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

/// Not derived: a derived `Default` left every channel's `index` at 0, so
/// all DMA completion interrupts were raised as DMA0 (and the ARM7 ones as
/// Timer1), and games waiting for a DMA1-3 interrupt hung.
impl Default for NDSDma {
    fn default() -> Self {
        Self::new()
    }
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
            dma.index = i as u32;
            dma.cnt.set(0, dma.is_arm9);
            dma.in_progress = false;
        }
    }

    // moved emulator/dma.rs:
    // pub fn dma_event(&mut self, index: u32);

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
            true => self.dmas[index].word_count as u16, // word count (lower halfword)
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
            dma.source = (dma.source & 0xFFFF_0000) | u32::from(lo); // replace the low halfword
        }
    }

    /// Write the high halfword of a DMA channel's source address (`DMAxSAD` bits 16-31).
    pub fn write_source_hi(&mut self, index: usize, hi: u16) {
        if index < 8 {
            let dma = &mut self.dmas[index];
            dma.source = (dma.source & 0x0000_FFFF) | (u32::from(hi) << 16); // replace the high halfword
        }
    }

    /// Write the low halfword of a DMA channel's destination address (`DMAxDAD` bits 0-15).
    pub fn write_dest_lo(&mut self, index: usize, lo: u16) {
        if index < 8 {
            let dma = &mut self.dmas[index];
            dma.destination = (dma.destination & 0xFFFF_0000) | u32::from(lo); // replace the low halfword
        }
    }

    /// Write the high halfword of a DMA channel's destination address (`DMAxDAD` bits 16-31).
    pub fn write_dest_hi(&mut self, index: usize, hi: u16) {
        if index < 8 {
            let dma = &mut self.dmas[index];
            dma.destination = (dma.destination & 0x0000_FFFF) | (u32::from(hi) << 16); // replace the high halfword
        }
    }

    /// Write DMAxCNT_L (word count bits 0-15). The count is latched when
    /// a transfer starts.
    pub fn write_len(&mut self, index: usize, len: u16) {
        if let Some(dma) = self.dmas.get_mut(index) {
            dma.word_count = u32::from(len);
        }
    }

    // moved emulator/dma.rs:
    // pub fn write_cnt(&mut self, index: usize, cnt: u16);
    // pub fn write_len_cnt(&mut self, index: usize, word: u32);
    // pub fn hblank_request(&mut self);
    // pub fn gamecart_request(&mut self);
    // pub fn gfxfifo_request(&mut self);
}
