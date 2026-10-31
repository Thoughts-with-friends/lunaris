//! One of the 16 sound channels: sample fetch through the channel FIFO
//! (PCM8 / PCM16 / ADPCM), PSG square waves (channels 8-13) and noise
//! (channels 14-15). See GBATEK "DS Sound Channels".
use crate::{Bus, Interpolation, adpcm};

/// Duty cycles 0-7 of the PSG channels (eighths high).
const PSG_TABLE: [[i16; 8]; 8] = {
    // DS Sound Channels 0..15: https://problemkaputt.de/gbatek.htm#dssoundchannels015
    let mut table = [[-0x7FFF; 8]; 8]; // PSG low level
    let mut duty = 0;
    while duty < 7 {
        let mut step = 7 - duty;
        while step < 8 {
            table[duty][step] = 0x7FFF; // PSG high level
            step += 1;
        }
        duty += 1;
    }
    table
};

/// One sound channel (SOUNDxCNT/SAD/TMR/PNT/LEN at 0x04000400 + x * 0x10).
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
pub(crate) struct Channel {
    pub(crate) num: usize,
    pub(crate) cnt: u32,
    pub(crate) src: u32,
    pub(crate) timer_reload: u16,
    /// Loop start in bytes.
    pub(crate) loop_pos: u32,
    /// Length after the loop start, in bytes.
    pub(crate) length: u32,

    pub(crate) volume: i32,
    pub(crate) volume_shift: u32,
    pub(crate) pan: i32,
    pub(crate) key_on: bool,

    pub(crate) timer: u32,
    pub(crate) pos: i32,
    pub(crate) cur_sample: i16,
    /// Previous samples (newest first) for interpolation.
    pub(crate) prev: [i16; 3],
    pub(crate) noise: u16,

    pub(crate) adpcm_val: i32,
    pub(crate) adpcm_index: i32,
    pub(crate) adpcm_val_loop: i32,
    pub(crate) adpcm_index_loop: i32,
    pub(crate) adpcm_byte: u8,

    /// 32-byte sound FIFO (melonDS models the hardware buffering).
    pub(crate) fifo: [u8; 32],
    pub(crate) fifo_read: usize,
    pub(crate) fifo_write: usize,
    pub(crate) fifo_offset: u32,
    pub(crate) fifo_level: i32,
}

impl Channel {
    /// SOUNDxCNT write: volume, shift, pan, format; a 0 -> 1 start bit sets key_on.
    pub(crate) fn set_cnt(&mut self, value: u32) {
        let old = self.cnt;
        self.cnt = value & 0xFF7F_837F; // SOUNDxCNT writable bits
        self.volume = (self.cnt & 0x7F) as i32; // volume multiplier (bits 0-6)
        if self.volume == 127 {
            self.volume = 128;
        }
        self.volume_shift = [4, 3, 2, 0][((self.cnt >> 8) & 3) as usize];
        self.pan = ((self.cnt >> 16) & 0x7F) as i32; // panning (bits 16-22)
        if self.pan == 127 {
            self.pan = 128;
        }
        // DS Sound Channels 0..15: https://problemkaputt.de/gbatek.htm#dssoundchannels015
        if value & (1 << 31) != 0 && old & (1 << 31) == 0 {
            // SOUNDxCNT bit 31: start (0 -> 1 edge)
            self.key_on = true;
        }
    }

    /// Sample format: 0 PCM8, 1 PCM16, 2 ADPCM, 3 PSG/noise.
    pub(crate) const fn format(&self) -> u32 {
        (self.cnt >> 29) & 3
    }

    /// Repeat mode: 1 loop, 2 one-shot.
    pub(crate) const fn repeat(&self) -> u32 {
        (self.cnt >> 27) & 3
    }

    /// Key on (first run() after start): reload the timer, reset position and prefill the FIFO.
    pub(crate) fn start(&mut self, bus: &mut Bus) {
        self.timer = u32::from(self.timer_reload);
        self.pos = if self.format() == 3 { -1 } else { -3 };
        self.noise = 0x7FFF; // noise LFSR seed
        self.prev = [0; 3];
        self.cur_sample = 0;
        self.fifo_read = 0;
        self.fifo_write = 0;
        self.fifo_offset = 0;
        self.fifo_level = 0;
        if self.format() != 3 {
            self.fifo_fill(bus);
            self.fifo_fill(bus);
        }
    }

    /// Reads up to 16 bytes of sample data from memory into the channel FIFO.
    pub(crate) fn fifo_fill(&mut self, bus: &mut Bus) {
        let total = self.loop_pos + self.length;
        if self.fifo_offset >= total {
            match self.repeat() {
                r if r & 1 != 0 => self.fifo_offset = self.loop_pos,
                r if r & 2 != 0 => return,
                _ => {}
            }
        }
        let burst = 16.min(total.saturating_sub(self.fifo_offset));
        for _ in (0..burst).step_by(4) {
            let addr = self.src.wrapping_add(self.fifo_offset);
            // Sound DMA cannot read the ARM7 BIOS.
            let word = if addr >= 0x4000 { bus.read32(addr) } else { 0 };
            for b in word.to_le_bytes() {
                self.fifo[self.fifo_write] = b;
                self.fifo_write = (self.fifo_write + 1) & 31;
            }
            self.fifo_offset += 4;
        }
        self.fifo_level += burst as i32;
    }

    /// Takes `bytes` bytes from the FIFO, refilling it when half empty.
    pub(crate) fn fifo_read(&mut self, bytes: usize, bus: &mut Bus) -> u32 {
        let mut value = 0;
        for i in 0..bytes {
            value |= u32::from(self.fifo[self.fifo_read]) << (i * 8);
            self.fifo_read = (self.fifo_read + 1) & 31;
        }
        self.fifo_level -= bytes as i32;
        if self.fifo_level <= 16 {
            self.fifo_fill(bus);
        }
        value
    }

    /// Handles the end of the sample data. Returns `false` if the channel
    /// stopped (one-shot).
    pub(crate) fn wrap_or_stop(&mut self, restart_pos: i32) -> bool {
        let repeat = self.repeat();
        if repeat & 1 != 0 {
            self.pos = restart_pos;
            true
        } else if repeat & 2 != 0 {
            self.cur_sample = 0;
            self.cnt &= !(1 << 31); // SOUNDxCNT bit 31: stop
            false
        } else {
            true
        }
    }

    /// Timer overflow: next signed 8-bit sample.
    pub(crate) fn next_pcm8(&mut self, bus: &mut Bus) {
        self.pos += 1;
        if self.pos < 0 {
            return;
        }
        if self.pos as u32 >= self.loop_pos + self.length
            && !self.wrap_or_stop(self.loop_pos as i32)
        {
            return;
        }
        self.cur_sample = i16::from(self.fifo_read(1, bus) as u8 as i8) << 8;
    }

    /// Timer overflow: next signed 16-bit sample.
    pub(crate) fn next_pcm16(&mut self, bus: &mut Bus) {
        self.pos += 1;
        if self.pos < 0 {
            return;
        }
        if (self.pos as u32) << 1 >= self.loop_pos + self.length
            && !self.wrap_or_stop((self.loop_pos >> 1) as i32)
        {
            return;
        }
        self.cur_sample = self.fifo_read(2, bus) as u16 as i16;
    }

    /// Timer overflow: header at the start, then the next 4-bit IMA-ADPCM nibble.
    pub(crate) fn next_adpcm(&mut self, bus: &mut Bus) {
        self.pos += 1;
        if self.pos < 8 {
            if self.pos == 0 {
                let header = self.fifo_read(4, bus);
                self.adpcm_val = i32::from(header as u16 as i16);
                self.adpcm_index = ((header >> 16) & 0x7F).min(88) as i32; // ADPCM header: initial table index (bits 16-22), max 88
                self.adpcm_val_loop = self.adpcm_val;
                self.adpcm_index_loop = self.adpcm_index;
            }
            return;
        }

        if (self.pos as u32) >> 1 >= self.loop_pos + self.length {
            if self.repeat() & 1 != 0 {
                self.pos = (self.loop_pos << 1) as i32;
                self.adpcm_val = self.adpcm_val_loop;
                self.adpcm_index = self.adpcm_index_loop;
                self.adpcm_byte = self.fifo_read(1, bus) as u8;
            } else if self.repeat() & 2 != 0 {
                self.cur_sample = 0;
                self.cnt &= !(1 << 31); // SOUNDxCNT bit 31: stop
                return;
            }
        } else {
            if self.pos & 1 == 0 {
                self.adpcm_byte = self.fifo_read(1, bus) as u8;
            } else {
                self.adpcm_byte >>= 4;
            }

            adpcm::decode_nibble(self.adpcm_byte, &mut self.adpcm_val, &mut self.adpcm_index);

            if self.pos as u32 == self.loop_pos << 1 {
                self.adpcm_val_loop = self.adpcm_val;
                self.adpcm_index_loop = self.adpcm_index;
            }
        }
        self.cur_sample = self.adpcm_val as i16;
    }

    /// Timer overflow: next step of the square wave (duty from SOUNDxCNT bits 24-26).
    pub(crate) fn next_psg(&mut self) {
        self.pos += 1;
        self.cur_sample = PSG_TABLE[((self.cnt >> 24) & 7) as usize][(self.pos & 7) as usize];
    }

    /// Timer overflow: next bit of the 15-bit LFSR noise.
    pub(crate) fn next_noise(&mut self) {
        if self.noise & 1 != 0 {
            self.noise = (self.noise >> 1) ^ 0x6000; // noise LFSR: carry out -> XOR 6000h
            self.cur_sample = -0x7FFF; // noise low level
        } else {
            self.noise >>= 1;
            self.cur_sample = 0x7FFF; // noise high level
        }
    }

    /// Advances the channel by `cycles` SPU cycles (16.76 MHz) and returns
    /// its volume-scaled output.
    pub(crate) fn run(&mut self, cycles: u32, bus: &mut Bus, interp: Interpolation) -> i32 {
        if self.cnt & (1 << 31) == 0 {
            // SOUNDxCNT bit 31: channel stopped
            return 0;
        }
        // 0 = PCM8, 1 = PCM16, 2 = ADPCM, 3 = PSG (ch 8-13) / noise (ch 14-15).
        let kind = match self.format() {
            3 if (8..14).contains(&self.num) => 3,
            3 if self.num >= 14 => 4,
            3 => return 0,
            f => f,
        };
        if kind < 3 && self.loop_pos + self.length < 16 {
            return 0;
        }
        if self.key_on {
            self.start(bus);
            self.key_on = false;
        }

        self.timer += cycles;
        while self.timer >> 16 != 0 {
            self.timer = u32::from(self.timer_reload) + (self.timer - 0x1_0000); // timer counts up and overflows at 10000h
            if kind < 3 && interp != Interpolation::None {
                self.prev = [self.cur_sample, self.prev[0], self.prev[1]];
            }
            match kind {
                0 => self.next_pcm8(bus),
                1 => self.next_pcm16(bus),
                2 => self.next_adpcm(bus),
                3 => self.next_psg(),
                _ => self.next_noise(),
            }
            if self.cnt & (1 << 31) == 0 {
                // SOUNDxCNT bit 31: channel stopped
                break;
            }
        }

        let sample = if kind < 3 && interp != Interpolation::None {
            // progress towards the next sample, 0..=FFh
            let reload = u32::from(self.timer_reload);
            let pos = (self.timer.saturating_sub(reload) * 0x100) / (0x1_0000 - reload).max(1);
            interp.apply(self.cur_sample, self.prev, pos as i32)
        } else {
            i32::from(self.cur_sample)
        };
        (sample << self.volume_shift) * self.volume
    }

    /// Adds the sample to left/right with the channel panning.
    pub(crate) fn pan_output(&self, sample: i32, left: &mut i32, right: &mut i32) {
        *left += ((i64::from(sample) * i64::from(128 - self.pan)) >> 10) as i32;
        *right += ((i64::from(sample) * i64::from(self.pan)) >> 10) as i32;
    }
}
