//! Optional sample interpolation (an emulation improvement, not a hardware
//! feature; melonDS `AudioInterpolation`).
//!
//! ```text
//!  PCM / ADPCM channel:  ... p2  p1  p0  cur        (last 4 fetched samples)
//!                                     |<-->|  pos = progress of the channel
//!                                             timer towards the next sample
//!  None    cur                        Linear  p0 + (cur - p0) * pos
//!  Cosine  same, with a cosine ramp   Cubic   4-point Hermite-like curve
//! ```
//! The output is delayed by about one sample, like melonDS.
use std::sync::LazyLock;

/// Interpolation applied to PCM8 / PCM16 / ADPCM channels.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Interpolation {
    #[default]
    None,
    Linear,
    Cosine,
    Cubic,
}

/// `(1 - cos(i * pi / 255)) * 2000h`, 1:1:14 fixed point.
static COS: LazyLock<[i32; 256]> = LazyLock::new(|| {
    core::array::from_fn(|i| {
        ((1.0 - (i as f64 * core::f64::consts::PI / 255.0).cos()) * 8192.0) as i32
    })
});

/// Cubic weights for (p2, p1, p0, cur) at each position.
static CUBIC: LazyLock<[[i32; 4]; 256]> = LazyLock::new(|| {
    core::array::from_fn(|i| {
        let i = i as i32;
        let (i1, i2, i3) = (i << 6, (i * i) >> 2, (i * i * i) >> 10);
        [-i3 + 2 * i2 - i1, i3 - 2 * i2 + 0x4000, -i3 + i2 + i1, i3 - i2]
    })
});

impl Interpolation {
    /// Interpolated value between `prev` (p0, p1, p2) and `cur` at
    /// position `pos` (0..=FFh).
    pub(crate) fn apply(self, cur: i16, prev: [i16; 3], pos: i32) -> i32 {
        let (cur, p0, p1, p2) =
            (i32::from(cur), i32::from(prev[0]), i32::from(prev[1]), i32::from(prev[2]));
        let pos = pos.clamp(0, 0xFF) as usize;
        match self {
            Self::None => cur,
            Self::Linear => (cur * pos as i32 + p0 * (0xFF - pos as i32)) >> 8,
            Self::Cosine => (cur * COS[pos] + p0 * COS[0xFF - pos]) >> 14,
            Self::Cubic => {
                let w = CUBIC[pos];
                (p2 * w[0] + p1 * w[1] + p0 * w[2] + cur * w[3]) >> 14
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints() {
        // Linear / cosine go from p0 to cur.
        for m in [Interpolation::Linear, Interpolation::Cosine] {
            let at_end = m.apply(1000, [0, 0, 0], 0xFF);
            assert!((990..=1000).contains(&at_end), "{m:?} {at_end}");
            let at_start = m.apply(1000, [500, 500, 500], 0);
            assert!((495..=505).contains(&at_start), "{m:?} {at_start}");
        }
        // Cubic goes from p1 to p0 (one more sample of delay).
        let c = Interpolation::Cubic;
        assert_eq!(c.apply(0, [0, 700, 0], 0), 700);
        assert!((690..=700).contains(&c.apply(0, [700, 0, 0], 0xFF)));
    }
}
