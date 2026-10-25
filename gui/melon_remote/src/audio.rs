//! Sending the console's sound without spending a third of the link on it.
//!
//! # The problem
//!
//! melonDS hands its SPU output over at 48 kHz, interleaved stereo `i16`. That
//! is 192 KB every second — **1.5 Mbit/s** — for audio alone, against a picture
//! that costs around 4 Mbit/s. Over a VPN that is a third of the budget spent
//! on a signal whose useful bandwidth is nowhere near 24 kHz: the DS's own SPU
//! mixes at 32768 Hz and its instruments are mostly ADPCM samples well below
//! that (GBATEK, "DS Sound").
//!
//! # What is done about it
//!
//! The sound is **decimated before it is sent** and **resampled back up on the
//! machine that plays it**, which is where the quality is preserved:
//!
//! ```text
//! console 48 kHz ─→ [Downsampler] ─→ wire 24 kHz ─→ [the client's own
//!                    box average                     Resampler, to whatever
//!                    anti-aliasing]                  the sound card wants]
//! ```
//!
//! Halving the rate halves the bandwidth. What it costs is everything above
//! 12 kHz — cymbals lose a little air, and nothing else in a DS soundtrack is
//! up there at all.
//!
//! The client does not need its own upsampler: [`crate::audio::Resampler`]
//! already interpolates from an arbitrary source rate to the device rate, and
//! carries its seam state between batches so the joins are inaudible. The
//! transport rate travels **in each datagram** rather than being configured at
//! both ends, so a client is never left guessing and a host may change it
//! mid-session.
//!
//! # Why the averaging matters
//!
//! Dropping every other sample would be cheaper and wrong: everything above the
//! new Nyquist folds back down as aliasing, which on a game soundtrack is a
//! metallic ring that is far more objectionable than the missing treble. A box
//! average over the samples each output covers is the cheapest filter that
//! removes it, and costs one add per sample.

/// Decimates the console's output to the rate it travels at.
///
/// One per session, on the host. State is carried between calls, so a batch
/// boundary is not a discontinuity.
pub struct Downsampler {
    /// Sum of the source frames gathered for the output frame being formed.
    accumulator: [i32; 2],
    /// How many source frames are in [`Self::accumulator`].
    gathered: i32,
    /// Fractional position, in units of the source rate. An output frame is
    /// emitted whenever this passes it.
    phase: u32,
}

impl Default for Downsampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Downsampler {
    #[must_use]
    pub const fn new() -> Self {
        Self { accumulator: [0; 2], gathered: 0, phase: 0 }
    }

    /// Convert `samples` from `source` Hz to `target` Hz, appending to `out`.
    ///
    /// Both are interleaved stereo `i16`. A `target` at or above `source` is a
    /// straight copy — this only ever decimates, and asking it to interpolate
    /// would be asking it to invent detail the client can invent just as well
    /// and closer to the speakers.
    pub fn run(&mut self, samples: &[i16], source: u32, target: u32, out: &mut Vec<i16>) {
        if target >= source || source == 0 || target == 0 {
            out.extend_from_slice(samples);
            return;
        }
        for frame in samples.as_chunks::<2>().0 {
            self.accumulator[0] += i32::from(frame[0]);
            self.accumulator[1] += i32::from(frame[1]);
            self.gathered += 1;
            self.phase += target;
            if self.phase < source {
                continue;
            }
            self.phase -= source;
            // The box average: every source frame this output covers,
            // weighted equally. `gathered` is never zero here, since it was
            // just incremented.
            out.push((self.accumulator[0] / self.gathered) as i16);
            out.push((self.accumulator[1] / self.gathered) as i16);
            self.accumulator = [0; 2];
            self.gathered = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Downsampler;

    /// Interleaved stereo where both channels hold the same value, so a test
    /// can reason about one number per frame.
    fn mono(values: &[i16]) -> Vec<i16> {
        values.iter().flat_map(|v| [*v, *v]).collect()
    }

    #[test]
    fn halving_the_rate_halves_the_frames() {
        let mut down = Downsampler::new();
        let mut out = Vec::new();
        down.run(&mono(&[0; 480]), 48_000, 24_000, &mut out);
        assert_eq!(out.len() / 2, 240, "480 frames at half rate must be 240");
    }

    #[test]
    fn a_third_of_the_rate_gives_a_third_of_the_frames() {
        let mut down = Downsampler::new();
        let mut out = Vec::new();
        down.run(&mono(&[0; 480]), 48_000, 16_000, &mut out);
        assert_eq!(out.len() / 2, 160);
    }

    /// The seam is what carries the quality: resampling that restarts on every
    /// batch loses a fractional sample each time and drifts.
    #[test]
    fn the_frame_count_holds_across_many_batches() {
        let mut down = Downsampler::new();
        let mut out = Vec::new();
        // 100 batches of a frame's worth, at a rate that does not divide evenly.
        for _ in 0..100 {
            down.run(&mono(&[0; 801]), 48_000, 22_050, &mut out);
        }
        let expected = (801.0 * 100.0 * 22_050.0 / 48_000.0) as usize;
        let got = out.len() / 2;
        assert!(
            got.abs_diff(expected) <= 2,
            "{got} frames out where {expected} were expected — the phase is drifting"
        );
    }

    /// A steady tone must keep its level. An average that divided by the wrong
    /// count would quietly halve the volume.
    #[test]
    fn a_constant_signal_keeps_its_level() {
        let mut down = Downsampler::new();
        let mut out = Vec::new();
        down.run(&mono(&[10_000; 960]), 48_000, 24_000, &mut out);
        assert!(
            out.iter().all(|s| (9_990..=10_010).contains(s)),
            "the level moved: {:?}",
            &out[..4]
        );
    }

    /// The point of averaging rather than dropping: the highest frequency
    /// there is — alternating full-scale samples — must come out near silence
    /// rather than folding down into an audible tone.
    #[test]
    fn the_averaging_removes_what_would_otherwise_alias() {
        let mut down = Downsampler::new();
        let mut out = Vec::new();
        let nyquist: Vec<i16> =
            (0..960).map(|i| if i % 2 == 0 { 20_000 } else { -20_000 }).collect();
        down.run(&mono(&nyquist), 48_000, 24_000, &mut out);
        let loudest = out.iter().map(|s| s.abs()).max().unwrap_or(0);
        assert!(loudest < 2_000, "aliasing came through at {loudest}, so it was not filtered");
    }

    /// Both channels have to stay their own. A shared accumulator would turn
    /// stereo into mono without anything else noticing.
    #[test]
    fn the_channels_stay_separate() {
        let mut down = Downsampler::new();
        let mut out = Vec::new();
        let stereo: Vec<i16> = (0..960).flat_map(|_| [8_000i16, -8_000]).collect();
        down.run(&stereo, 48_000, 24_000, &mut out);
        for frame in out.as_chunks::<2>().0 {
            assert!(frame[0] > 7_000, "the left channel lost its level: {frame:?}");
            assert!(frame[1] < -7_000, "the right channel lost its level: {frame:?}");
        }
    }

    #[test]
    fn a_target_at_or_above_the_source_is_a_copy() {
        let mut down = Downsampler::new();
        let mut out = Vec::new();
        let samples = mono(&[1, 2, 3, 4]);
        down.run(&samples, 48_000, 48_000, &mut out);
        assert_eq!(out, samples);
        out.clear();
        down.run(&samples, 48_000, 96_000, &mut out);
        assert_eq!(out, samples);
    }
}
