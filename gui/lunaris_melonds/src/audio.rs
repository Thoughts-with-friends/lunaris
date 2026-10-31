//! The core's SPU output (about 32728 Hz: 33.51 MHz / 1024) resampled to
//! the 48 kHz melon_egui plays (melonds' `SPU_SAMPLE_RATE`).
use std::collections::VecDeque;

/// Rate the core produces.
const IN_RATE: f64 = ds_spu::OUTPUT_RATE_HZ;
/// Rate the front end expects.
const OUT_RATE: f64 = 48_000.0;
/// At most this many output pairs are held (about a second).
const MAX_QUEUED: usize = 48_000;

/// Linear-interpolating stereo resampler with an output queue.
#[derive(Default)]
pub(crate) struct Resampler {
    /// Position between `prev` and the next input pair, 0..1.
    phase: f64,
    prev: (f64, f64),
    out: VecDeque<(i16, i16)>,
}

impl Resampler {
    /// Feeds interleaved stereo input.
    pub(crate) fn push(&mut self, input: &[i16]) {
        let step = IN_RATE / OUT_RATE;
        for pair in input.as_chunks::<2>().0 {
            let cur = (f64::from(pair[0]), f64::from(pair[1]));
            while self.phase < 1.0 {
                let t = self.phase;
                let l = (cur.0 - self.prev.0).mul_add(t, self.prev.0);
                let r = (cur.1 - self.prev.1).mul_add(t, self.prev.1);
                self.out.push_back((l as i16, r as i16));
                self.phase += step;
            }
            self.phase -= 1.0;
            self.prev = cur;
        }
        while self.out.len() > MAX_QUEUED {
            self.out.pop_front();
        }
    }

    /// Output pairs waiting.
    pub(crate) fn queued(&self) -> usize {
        self.out.len()
    }

    /// Moves pairs into `out` (interleaved); returns the pairs written.
    pub(crate) fn pop(&mut self, out: &mut [i16]) -> usize {
        let n = (out.len() / 2).min(self.out.len());
        for (dst, (l, r)) in out.as_chunks_mut::<2>().0.iter_mut().zip(self.out.drain(..n)) {
            dst[0] = l;
            dst[1] = r;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_is_converted() {
        let mut r = Resampler::default();
        let input = vec![1000_i16; 32728 * 2];
        r.push(&input);
        let n = r.queued();
        assert!((47_900..=48_100).contains(&n), "{n}");
        let mut out = vec![0; 8];
        assert_eq!(r.pop(&mut out), 4);
        assert_eq!(out[6], 1000);
    }
}
