//! Mixing: runs the channels for one output sample, applies panning,
//! the capture units and the master volume. See GBATEK "DS Sound Control".
use crate::{Bus, CYCLES_PER_SAMPLE, SAMPLE_RATE, Spu, channel::Channel};

impl Spu {
    /// Produces one output sample covering `CYCLES_PER_SAMPLE` bus cycles
    /// and appends it to the sample buffer. `bus` reads sample data.
    ///
    /// ```text
    /// from: ds_emu run_spu(), once per 1024 ARM7 cycles
    ///   |
    ///   v
    /// mix(bus)
    ///   1. SOUNDCNT master enable off -> silence
    ///   2. channel[0..15].run(512 SPU cycles)  timer steps fetch PCM8 /
    ///      PCM16 / ADPCM through the channel FIFO, or PSG / noise
    ///   3. pan_output() into left/right (channels 1/3 may be excluded,
    ///      SOUNDCNT bits 12-13)
    ///   4. capture 0/1 enabled -> capture.run(left/right) -> memory
    ///   5. output select (SOUNDCNT bits 8-11): mixer or channel 1/3
    ///   6. master volume, clamp to i16, push the stereo pair
    ///   |
    ///   v
    /// to:   take_samples() (ds_emu take_audio_samples -> frontend audio)
    /// ```
    pub fn mix(&mut self, bus: &mut Bus) {
        // Channel timers run at half the bus clock.
        let cycles = CYCLES_PER_SAMPLE / 2;
        let (mut left, mut right) = (0_i32, 0_i32);
        let (mut left_out, mut right_out) = (0_i32, 0_i32);

        // DS Sound Control Registers: https://problemkaputt.de/gbatek.htm#dssoundcontrolregisters
        if self.soundcnt & (1 << 15) != 0 {
            // SOUNDCNT bit 15: master enable
            let mut out = [0_i32; 16];
            for (o, ch) in out.iter_mut().zip(self.channels.iter_mut()) {
                *o = ch.run(cycles, bus, self.interpolation);
            }
            for (i, &o) in out.iter().enumerate() {
                // SOUNDCNT bits 12/13: channel 1/3 not sent to the mixer.
                if (i == 1 && self.soundcnt & (1 << 12) != 0)
                    || (i == 3 && self.soundcnt & (1 << 13) != 0)
                // SOUNDCNT bit 13: channel 3 not output to the mixer
                {
                    continue;
                }
                self.channels[i].pan_output(o, &mut left, &mut right);
            }

            // Sound capture of the left / right mixer.
            for (i, mixed) in [left, right].into_iter().enumerate() {
                // DS Sound Capture: https://problemkaputt.de/gbatek.htm#dssoundcapture
                if self.captures[i].cnt & 0x80 != 0 {
                    // SNDCAPxCNT bit 7: capture running
                    let sample = (mixed >> 8).clamp(-0x8000, 0x7FFF); // clamp to signed 16 bits
                    self.captures[i].run(cycles, sample, bus);
                }
            }

            let side = |ch: &Channel, sample: i32, right_side: bool| {
                let pan = if right_side { ch.pan } else { 128 - ch.pan };
                ((i64::from(sample) * i64::from(pan)) >> 10) as i32
            };
            let (ch1, ch3) = (&self.channels[1], &self.channels[3]);
            left_out = match (self.soundcnt >> 8) & 3 {
                0 => left,
                1 => side(ch1, out[1], false),
                2 => side(ch3, out[3], false),
                _ => side(ch1, out[1], false) + side(ch3, out[3], false),
            };
            right_out = match (self.soundcnt >> 10) & 3 {
                0 => right,
                1 => side(ch1, out[1], true),
                2 => side(ch3, out[3], true),
                _ => side(ch1, out[1], true) + side(ch3, out[3], true),
            };
        }

        // DS Sound Control Registers: https://problemkaputt.de/gbatek.htm#dssoundcontrolregisters
        let mut master = i64::from(self.soundcnt & 0x7F); // SOUNDCNT master volume (bits 0-6)
        if master == 127 {
            master = 128;
        }
        let finish = |v: i32| -> i16 {
            // The SOUNDBIAS used by commercial games (0x200) is the DC
            // offset of an unsigned DAC; it is removed here.
            let v = ((i64::from(v) * master) >> 7) >> 8;
            v.clamp(-0x8000, 0x7FFF) as i16 // clamp to signed 16 bits
        };
        // Without a consumer (headless use) keep at most ~2 s of audio.
        if self.samples.len() >= SAMPLE_RATE as usize * 4 {
            self.samples.drain(..SAMPLE_RATE as usize * 2);
        }
        self.samples.push(finish(left_out));
        self.samples.push(finish(right_out));
    }

    /// Takes the buffered interleaved stereo samples (32768 Hz); from
    /// ds_emu take_audio_samples() after each frame.
    pub fn take_samples(&mut self, out: &mut Vec<i16>) {
        out.append(&mut self.samples);
    }

    /// Drops buffered samples (e.g. when no audio output is attached).
    pub fn clear_samples(&mut self) {
        self.samples.clear();
    }
}
