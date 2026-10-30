//! The two sound capture units, which record the left / right mixer output
//! back to memory. See GBATEK "DS Sound Capture".
use crate::Bus;

/// Sound capture unit (SNDCAPxCNT/DAD/LEN, melonDS `SPUCaptureUnit`).
/// Capture 0 records the left mixer, capture 1 the right one, clocked by the
/// timer of channel 1 / channel 3.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
pub(crate) struct Capture {
    pub(crate) cnt: u8,
    pub(crate) dst: u32,
    /// Length in bytes.
    pub(crate) length: u32,
    pub(crate) timer_reload: u16,
    pub(crate) timer: u32,
    pub(crate) pos: u32,
    pub(crate) fifo: [u8; 16],
    pub(crate) fifo_read: usize,
    pub(crate) fifo_write: usize,
    pub(crate) write_offset: u32,
    pub(crate) level: u32,
}

impl Capture {
    /// SNDCAPxCNT write; starting a capture resets its position and FIFO.
    pub(crate) fn set_cnt(&mut self, value: u8) {
        // DS Sound Capture: https://problemkaputt.de/gbatek.htm#dssoundcapture
        if value & 0x80 != 0 && self.cnt & 0x80 == 0 {
            // SNDCAPxCNT bit 7: start (0 -> 1 edge)
            self.timer = u32::from(self.timer_reload);
            self.pos = 0;
            self.fifo_read = 0;
            self.fifo_write = 0;
            self.write_offset = 0;
            self.level = 0;
        }
        let mut value = value & 0x8F; // writable bits: 0-3 and 7
        if value & 0x80 == 0 {
            // bit 7 clear: capture stopped
            value &= !1;
        }
        self.cnt = value;
    }

    /// Writes the buffered words to memory (wrapping at the capture length).
    pub(crate) fn flush(&mut self, bus: &mut Bus) {
        for _ in 0..4 {
            let i = self.fifo_read * 4;
            let word = u32::from_le_bytes([
                self.fifo[i],
                self.fifo[i + 1],
                self.fifo[i + 2],
                self.fifo[i + 3],
            ]);
            bus.write32(self.dst.wrapping_add(self.write_offset), word);
            self.fifo_read = (self.fifo_read + 1) & 3;
            self.level = self.level.saturating_sub(4);
            self.write_offset += 4;
            if self.write_offset >= self.length {
                self.write_offset = 0;
                break;
            }
        }
    }

    /// Buffers captured bytes and flushes every 16 bytes.
    pub(crate) fn push(&mut self, bytes: &[u8], bus: &mut Bus) {
        for &b in bytes {
            self.fifo[self.fifo_write] = b;
            self.fifo_write = (self.fifo_write + 1) & 0xF; // 16-byte capture FIFO index
        }
        self.level += bytes.len() as u32;
        if self.level >= 16 {
            self.flush(bus);
        }
    }

    /// Advances by `cycles` (timer of channel 1/3); on each overflow
    /// records `sample` as PCM8/PCM16, looping or stopping at the end.
    pub(crate) fn run(&mut self, cycles: u32, sample: i32, bus: &mut Bus) {
        self.timer += cycles;
        let pcm8 = self.cnt & 0x08 != 0; // bit 3: PCM8 format (else PCM16)
        while self.timer >> 16 != 0 {
            self.timer = u32::from(self.timer_reload) + (self.timer - 0x1_0000); // timer counts up and overflows at 10000h
            if pcm8 {
                self.push(&[(sample >> 8) as u8], bus);
                self.pos += 1;
            } else {
                self.push(&(sample as i16).to_le_bytes(), bus);
                self.pos += 2;
            }
            if self.pos >= self.length {
                if self.level >= 4 {
                    self.flush(bus);
                }
                if self.cnt & 0x04 != 0 {
                    // bit 2: one-shot (else loop)
                    self.cnt &= 0x7F; // one-shot: done
                    return;
                }
                self.pos = 0;
            }
        }
    }
}
