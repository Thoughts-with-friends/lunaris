//! Post-process upscaling of the finished RGBA8 screen buffers.
//!
//! Deliberately operates on the already-composited RGBA8 frame produced by
//! [`crate::framebuffer::abgr1555_to_rgba8`], not on any `core` state — see
//! `docs/design/resolution-upscaling-design.md` §2 for why raising the
//! *internal* NDS resolution is out of scope and why post-processing the
//! finished frame is the safe place for this feature.

use serde::{Deserialize, Serialize};

/// Selects which upscaling algorithm is applied to the screen textures.
///
/// `None` is the default and preserves today's behavior exactly (texture
/// filtering only, no extra buffer, no extra copy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpscaleMethod {
    #[default]
    None,
    /// Edge-directed pixel-art upscaler. See
    /// `docs/design/resolution-upscaling-design.md` §4.
    Xbrz,
}

/// Smallest and largest user-selectable scale factor (1 = native, 16 = max).
pub const MIN_FACTOR: u8 = 1;
pub const MAX_FACTOR: u8 = 16;

/// Clamps a (possibly hand-edited) config value into the supported range.
pub const fn clamp_factor(factor: u8) -> u8 {
    if factor < MIN_FACTOR {
        MIN_FACTOR
    } else if factor > MAX_FACTOR {
        MAX_FACTOR
    } else {
        factor
    }
}

/// One resolved upscaling plan: a chain of xBRZ passes (each in 2..=6),
/// optionally followed by a bilinear downsample to reach an exact factor
/// that isn't itself a product of single-pass factors.
///
/// See `docs/design/resolution-upscaling-design.md` §5.2 for the derivation
/// of this table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpscalePlan {
    /// xBRZ passes to run in order, each factor in 2..=6.
    pub passes: Vec<u8>,
    /// Target factor after the optional final bilinear resample.
    pub target_factor: u8,
}

impl UpscalePlan {
    /// The factor produced right after the xBRZ pass chain, before any
    /// resample. Equal to `target_factor` when no resample is needed.
    fn xbrz_factor(&self) -> u8 {
        self.passes.iter().product()
    }

    /// Whether a bilinear resample step is needed after the xBRZ passes.
    fn needs_resample(&self) -> bool {
        self.xbrz_factor() != self.target_factor
    }
}

/// Computes the pass chain for a target factor in `MIN_FACTOR..=MAX_FACTOR`.
///
/// Pure and table-driven, see `docs/design/resolution-upscaling-design.md`
/// §5.2. `factor` is clamped internally so this never panics.
pub fn plan(factor: u8) -> UpscalePlan {
    let factor = clamp_factor(factor);
    let passes = match factor {
        1 => vec![],
        2 => vec![2],
        3 => vec![3],
        4 => vec![4],
        5 => vec![5],
        6 => vec![6],
        7 => vec![4, 2], // -> 8x, then resample down to 7x
        8 => vec![4, 2],
        9 => vec![3, 3],
        10 => vec![5, 2],
        11 => vec![4, 3], // -> 12x, then resample down to 11x
        12 => vec![4, 3],
        13 => vec![4, 4], // -> 16x, then resample down to 13x
        14 => vec![4, 4], // -> 16x, then resample down to 14x
        15 => vec![5, 3],
        16 => vec![4, 4],
        _ => unreachable!("factor clamped to 1..=16"),
    };
    UpscalePlan { passes, target_factor: factor }
}

/// Upscales one RGBA8 screen buffer according to `method`/`factor`.
///
/// `rgba` must be exactly `width * height * 4` bytes (as produced by
/// [`crate::framebuffer::abgr1555_to_rgba8`]). Returns the possibly-scaled
/// buffer plus its new `(width, height)`. For `UpscaleMethod::None` or
/// `factor == 1` this is a passthrough that returns `rgba` unchanged without
/// copying.
pub fn upscale(
    rgba: Vec<u8>,
    width: usize,
    height: usize,
    method: UpscaleMethod,
    factor: u8,
) -> (Vec<u8>, usize, usize) {
    let plan = plan(factor);
    if method == UpscaleMethod::None || plan.target_factor == 1 {
        return (rgba, width, height);
    }

    // xBRZ blends alpha at edges; the NDS screen buffer's per-pixel alpha
    // (0 or 0xFF, see `abgr1555_to_rgba8`) is a capture/transparency flag,
    // not a blend weight, so force full opacity before scaling to avoid
    // fringed transparent edges. See design doc §5.4.
    let mut opaque = rgba;
    for pixel in opaque.as_chunks_mut::<4>().0 {
        pixel[3] = 0xFF;
    }

    let (mut buf, mut w, mut h) = (opaque, width, height);
    for &pass in &plan.passes {
        buf = xbrz::scale_rgba(&buf, w, h, pass as usize);
        w *= pass as usize;
        h *= pass as usize;
    }

    if plan.needs_resample() {
        let target_w = width * plan.target_factor as usize;
        let target_h = height * plan.target_factor as usize;
        buf = resample_bilinear(&buf, w, h, target_w, target_h);
        w = target_w;
        h = target_h;
    }

    // xBRZ's border interpolation can still produce a non-0xFF alpha at the
    // image edge even though every input pixel was forced opaque above (its
    // blend weights are not alpha-aware). Force opacity again on the final
    // buffer so a fully-opaque source can never come out with translucent
    // edge pixels.
    for pixel in buf.as_chunks_mut::<4>().0 {
        pixel[3] = 0xFF;
    }

    (buf, w, h)
}

/// Simple bilinear resample used only to trim an xBRZ pass chain's overshoot
/// down to an exact requested factor (e.g. 8x -> 7x). Not a general-purpose
/// image resizer: callers are expected to only ever shrink slightly.
fn resample_bilinear(
    src: &[u8],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<u8> {
    let mut dst = vec![0u8; dst_w * dst_h * 4];
    let x_ratio = src_w as f32 / dst_w as f32;
    let y_ratio = src_h as f32 / dst_h as f32;

    for dy in 0..dst_h {
        let sy = (dy as f32 + 0.5) * y_ratio - 0.5;
        let sy0 = sy.floor().max(0.0) as usize;
        let sy1 = (sy0 + 1).min(src_h - 1);
        let ty = (sy - sy0 as f32).clamp(0.0, 1.0);

        for dx in 0..dst_w {
            let sx = (dx as f32 + 0.5) * x_ratio - 0.5;
            let sx0 = sx.floor().max(0.0) as usize;
            let sx1 = (sx0 + 1).min(src_w - 1);
            let tx = (sx - sx0 as f32).clamp(0.0, 1.0);

            let px = |x: usize, y: usize, c: usize| -> f32 { src[(y * src_w + x) * 4 + c] as f32 };

            let dst_idx = (dy * dst_w + dx) * 4;
            for c in 0..4 {
                let top = px(sx0, sy0, c) * (1.0 - tx) + px(sx1, sy0, c) * tx;
                let bottom = px(sx0, sy1, c) * (1.0 - tx) + px(sx1, sy1, c) * tx;
                let value = top * (1.0 - ty) + bottom * ty;
                dst[dst_idx + c] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_table_matches_design_doc() {
        let expected: [(u8, &[u8]); 16] = [
            (1, &[]),
            (2, &[2]),
            (3, &[3]),
            (4, &[4]),
            (5, &[5]),
            (6, &[6]),
            (7, &[4, 2]),
            (8, &[4, 2]),
            (9, &[3, 3]),
            (10, &[5, 2]),
            (11, &[4, 3]),
            (12, &[4, 3]),
            (13, &[4, 4]),
            (14, &[4, 4]),
            (15, &[5, 3]),
            (16, &[4, 4]),
        ];
        for (factor, passes) in expected {
            assert_eq!(plan(factor).passes, passes, "factor {factor}");
            assert_eq!(plan(factor).target_factor, factor);
        }
    }

    #[test]
    fn factor_is_clamped() {
        assert_eq!(clamp_factor(0), 1);
        assert_eq!(clamp_factor(1), 1);
        assert_eq!(clamp_factor(16), 16);
        assert_eq!(clamp_factor(200), 16);
    }

    fn synthetic_rgba(width: usize, height: usize) -> Vec<u8> {
        let mut buf = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let on = (x + y) % 2 == 0;
                let v = if on { 0xFF } else { 0x00 };
                buf.extend_from_slice(&[v, v, v, if on { 0xFF } else { 0x80 }]);
            }
        }
        buf
    }

    #[test]
    fn output_dimensions_match_factor_for_every_supported_factor() {
        let (w, h) = (8, 6);
        let src = synthetic_rgba(w, h);
        for factor in MIN_FACTOR..=MAX_FACTOR {
            let (out, out_w, out_h) = upscale(src.clone(), w, h, UpscaleMethod::Xbrz, factor);
            assert_eq!(out_w, w * factor as usize, "width at factor {factor}");
            assert_eq!(out_h, h * factor as usize, "height at factor {factor}");
            assert_eq!(out.len(), out_w * out_h * 4, "buffer length at factor {factor}");
        }
    }

    #[test]
    fn method_none_is_passthrough() {
        let (w, h) = (8, 6);
        let src = synthetic_rgba(w, h);
        let (out, out_w, out_h) = upscale(src.clone(), w, h, UpscaleMethod::None, 4);
        assert_eq!(out, src);
        assert_eq!((out_w, out_h), (w, h));
    }

    #[test]
    fn factor_one_is_passthrough_even_with_xbrz() {
        let (w, h) = (8, 6);
        let src = synthetic_rgba(w, h);
        let (out, out_w, out_h) = upscale(src.clone(), w, h, UpscaleMethod::Xbrz, 1);
        assert_eq!(out, src);
        assert_eq!((out_w, out_h), (w, h));
    }

    #[test]
    fn scaled_output_is_fully_opaque() {
        let (w, h) = (8, 6);
        let src = synthetic_rgba(w, h);
        let (out, _, _) = upscale(src, w, h, UpscaleMethod::Xbrz, 2);
        assert!(out.as_chunks::<4>().0.iter().all(|p| p[3] == 0xFF));
    }
}
