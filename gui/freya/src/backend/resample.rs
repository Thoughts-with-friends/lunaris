//! Sample rate conversion for the cores and the Remote Desktop stream.

/// Linear resampler for interleaved stereo `i16`.
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Position of the next output between `last` (0.0) and the first
    /// input frame (1.0).
    pos: f64,
    last: [i16; 2],
}

impl Resampler {
    pub const fn new(step: f64) -> Self {
        Self { step, pos: 1.0, last: [0; 2] }
    }

    pub fn process(&mut self, input: &[i16], out: &mut Vec<i16>) {
        let frames = input.as_chunks::<2>().0;
        // Frame `i` of the stream "last, frames..." is at index `i`.
        let at = |i: usize| if i == 0 { self.last } else { frames[i - 1] };
        let end = frames.len() as f64;
        #[allow(clippy::while_float)]
        while self.pos < end {
            let i = self.pos as usize;
            let t = self.pos - i as f64;
            let (a, b) = (at(i), at(i + 1));
            for c in 0..2 {
                let v = (f64::from(b[c]) - f64::from(a[c])).mul_add(t, f64::from(a[c]));
                out.push(v as i16);
            }
            self.pos += self.step;
        }
        if let Some(&last) = frames.last() {
            self.last = last;
            self.pos -= frames.len() as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Resampler;

    #[test]
    fn resamples_48k_to_the_spu_rate() {
        let mut r = Resampler::new(48_000.0 / 32_728.0);
        let mut out = Vec::new();
        // One second of a ramp, in 60 frame-sized chunks.
        let input: Vec<i16> = (0..48_000_i32).flat_map(|i| [(i % 1000) as i16; 2]).collect();
        for chunk in input.chunks(1600) {
            r.process(chunk, &mut out);
        }
        let frames = out.len() / 2;
        assert!((32_700..=32_760).contains(&frames), "{frames} frames");
        assert!(out.chunks(2).all(|f| f[0] == f[1]));
    }
}
