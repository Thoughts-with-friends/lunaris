//! Main memory display FIFO (DISP_MMEM_FIFO, 0x04000068).
//!
//! ```text
//!  main RAM --DMA (ARM9 timing 4, 4 words per request)--> 0x04000068
//!                                                            |
//!                         16-halfword ring (2 pixels per word write)
//!                                                            |
//!   every 8 pixels of a visible line: request a DMA, then take 8 pixels
//!                                                            v
//!                                  `line` (256 BGR555 pixels of this line)
//!                                     -> DISPCNT display mode 3
//!                                     -> DISPCAPCNT source B (bit 25)
//! ```
//!
//! GBATEK: [DS Video Capture and Main Memory Display Mode](https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode)

use ds_mem_addr::PIXELS_PER_LINE;

use crate::Gpu;

/// Pixels the FIFO feeds per DMA request.
pub const FIFO_STEP: usize = 8;

/// The display FIFO and the line it produced.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct DisplayFifo {
    fifo: [u16; 16],
    read: u8,
    write: u8,
    /// Pixels of the current line taken from the FIFO.
    #[serde(with = "serde_big_array::BigArray")]
    pub line: [u16; PIXELS_PER_LINE],
}

impl Default for DisplayFifo {
    fn default() -> Self {
        Self { fifo: [0; 16], read: 0, write: 0, line: [0; PIXELS_PER_LINE] }
    }
}

impl DisplayFifo {
    /// Write to 0x04000068 (first pixel of the pair).
    pub const fn write_lo(&mut self, value: u16) {
        self.fifo[self.write as usize] = value;
    }

    /// Write to 0x0400006A (second pixel); completes the pair.
    pub const fn write_hi(&mut self, value: u16) {
        self.fifo[(self.write as usize + 1) & 0xF] = value;
        self.write = (self.write + 2) & 0xF;
    }

    /// Takes `n` pixels into `line[offset..]`.
    pub fn sample(&mut self, offset: usize, n: usize) {
        for px in &mut self.line[offset..offset + n] {
            *px = self.fifo[self.read as usize];
            self.read = (self.read + 1) & 0xF;
        }
    }

    /// Empties the FIFO (VBlank).
    pub const fn reset(&mut self) {
        self.read = 0;
        self.write = 0;
    }
}

impl Gpu {
    /// Whether this line needs the FIFO: engine A shows main memory
    /// (display mode 3) or captures it (DISPCAPCNT bit 25 with a source
    /// that uses B).
    pub fn uses_display_fifo(&self) -> bool {
        let e = &self.engine_upper;
        e.dispcnt.display_mode == 3
            || (e.dispcapcnt.enable_busy
                && e.dispcapcnt.b_display_fifo
                && e.dispcapcnt.capture_source != 0)
    }
}
