//! xBRZ post-processing of the finished screens.
//!
//! The same feature lunaris's own front end has (`gui/common/src/upscale.rs`,
//! and `docs/design/resolution-upscaling-design.md` for the reasoning), brought
//! over so the two can be compared on equal terms — and, for this front end,
//! because it is the only thing that can improve the *2D* picture: raising the
//! internal resolution (see [`crate::video`]) rasterises 3D geometry at a
//! higher resolution, but a 2D layer is drawn from tiles at 256x192 whatever
//! the renderer does.
//!
//! # Where it applies
//!
//! Both renderers — but by two different routes, because the picture is in two
//! different places.
//!
//! Under the **software** renderer the frame arrives as host pixels at
//! 256x192, and this module filters them: cheap, because that is a fifth of a
//! megapixel, and it is the only chance the picture gets.
//!
//! Under an **OpenGL** renderer the frame never leaves the GPU, and the same
//! rule lives in the blit shader instead ([`crate::gl_screen`]). That is not a
//! shortcut but the better place for it: the filter then runs at the size the
//! picture is *drawn*, once per fragment the GPU was going to shade anyway, so
//! it costs no frame rate and puts no ceiling on the internal resolution. The
//! two settings stop competing — the internal resolution sharpens the 3D, the
//! shader sharpens the 2D, and neither takes anything from the other.
//!
//! Reading the frame back to run *this* code on it would do the opposite: at 4x
//! internal it is 1.5 million pixels a frame through a CPU filter, and the two
//! scales would multiply into an intermediate image that has to be capped. The
//! [`Method`] setting is shared, so a user turns xBRZ on once and gets whichever
//! of the two the current renderer can do.
//!
//! # Why the pass chain
//!
//! xBRZ takes one factor in `2..=6` per pass, so a factor outside that is a
//! chain (`8 = 4 x 2`), and a factor no chain lands on exactly is the next
//! chain up followed by a bilinear step down (`7 = 4 x 2`, resampled). The
//! table below is lunaris's, so both front ends scale a picture identically.

/// Which post-process filter runs on the finished screens.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum Method {
    /// No post-processing: the texture goes up at 256x192 and egui's own
    /// filtering decides what a magnified pixel looks like.
    #[default]
    None,
    /// Edge-directed pixel-art upscaler.
    Xbrz,
}

impl Method {
    pub const ALL: [Self; 2] = [Self::None, Self::Xbrz];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Xbrz => "xBRZ",
        }
    }
}

/// Smallest and largest factor the dialog offers; 1 is native.
pub const MIN_FACTOR: u8 = 1;
pub const MAX_FACTOR: u8 = 6;

/// Hold a hand-edited settings file to the supported range.
pub const fn clamp_factor(factor: u8) -> u8 {
    if factor < MIN_FACTOR {
        MIN_FACTOR
    } else if factor > MAX_FACTOR {
        MAX_FACTOR
    } else {
        factor
    }
}

/// The xBRZ passes for a target factor, and the factor they are resampled to
/// afterwards when the chain overshoots.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Plan {
    /// Passes to run in order, each factor in `2..=6`.
    pub passes: Vec<u8>,
    /// What the picture is scaled to once the optional resample has run.
    pub target_factor: u8,
}

impl Plan {
    /// The factor the pass chain alone produces.
    fn chain_factor(&self) -> u8 {
        self.passes.iter().product()
    }

    /// Whether the chain overshoots and has to be brought back down.
    fn needs_resample(&self) -> bool {
        self.chain_factor() != self.target_factor
    }
}

/// The pass chain for `factor`, clamped to `MIN_FACTOR..=MAX_FACTOR`.
pub fn plan(factor: u8) -> Plan {
    let factor = clamp_factor(factor);
    let passes = match factor {
        1 => vec![],
        2 => vec![2],
        3 => vec![3],
        4 => vec![4],
        5 => vec![5],
        _ => vec![6],
    };
    Plan { passes, target_factor: factor }
}

/// Scale one screen.
///
/// `rgba` is `width * height * 4` bytes. Returns the buffer and its new size;
/// [`Method::None`] and factor 1 hand the input straight back without copying.
pub fn upscale(
    rgba: Vec<u8>,
    width: usize,
    height: usize,
    method: Method,
    factor: u8,
) -> (Vec<u8>, usize, usize) {
    let plan = plan(factor);
    if method == Method::None || plan.target_factor == 1 {
        return (rgba, width, height);
    }

    // xBRZ blends alpha at edges. The DS's pixels are opaque as far as the
    // window is concerned, so forcing that first keeps the filter from
    // inventing translucent fringes.
    let mut buf = rgba;
    for pixel in buf.as_chunks_mut::<4>().0 {
        pixel[3] = 0xFF;
    }

    let (mut w, mut h) = (width, height);
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

    // The filter's border interpolation is not alpha-aware and can leave a
    // non-opaque edge pixel behind even though every input pixel was opaque.
    for pixel in buf.as_chunks_mut::<4>().0 {
        pixel[3] = 0xFF;
    }
    (buf, w, h)
}

/// Bilinear resample, used only to trim a pass chain's overshoot to the exact
/// factor asked for. Not a general image resizer.
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
        let sy = (dy as f32 + 0.5).mul_add(y_ratio, -0.5);
        let sy0 = sy.floor().max(0.0) as usize;
        let sy1 = (sy0 + 1).min(src_h - 1);
        let ty = (sy - sy0 as f32).clamp(0.0, 1.0);

        for dx in 0..dst_w {
            let sx = (dx as f32 + 0.5).mul_add(x_ratio, -0.5);
            let sx0 = sx.floor().max(0.0) as usize;
            let sx1 = (sx0 + 1).min(src_w - 1);
            let tx = (sx - sx0 as f32).clamp(0.0, 1.0);

            let px =
                |x: usize, y: usize, c: usize| -> f32 { f32::from(src[(y * src_w + x) * 4 + c]) };
            let dst_idx = (dy * dst_w + dx) * 4;
            for c in 0..4 {
                let top = px(sx1, sy0, c).mul_add(tx, px(sx0, sy0, c) * (1.0 - tx));
                let bottom = px(sx1, sy1, c).mul_add(tx, px(sx0, sy1, c) * (1.0 - tx));
                dst[dst_idx + c] =
                    bottom.mul_add(ty, top * (1.0 - ty)).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    dst
}

#[cfg(test)]
mod tests {
    use super::{MAX_FACTOR, Method, clamp_factor, plan, upscale};

    #[test]
    fn a_plan_multiplies_out_to_what_was_asked_for() {
        for factor in 1..=MAX_FACTOR {
            let plan = plan(factor);
            assert_eq!(plan.target_factor, factor);
            let product: u8 = plan.passes.iter().product();
            assert_eq!(product.max(1), factor, "chain for {factor}x");
        }
    }

    #[test]
    fn a_hand_edited_factor_is_held_to_the_range() {
        assert_eq!(clamp_factor(0), 1);
        assert_eq!(clamp_factor(99), MAX_FACTOR);
    }

    #[test]
    fn nothing_is_copied_when_there_is_nothing_to_do() {
        let src = vec![7u8; 4 * 4 * 4];
        let (out, w, h) = upscale(src.clone(), 4, 4, Method::None, 4);
        assert_eq!((w, h), (4, 4));
        assert_eq!(out, src);
        let (out, w, h) = upscale(src.clone(), 4, 4, Method::Xbrz, 1);
        assert_eq!((w, h), (4, 4));
        assert_eq!(out, src);
    }

    #[test]
    fn scaling_grows_the_picture_and_leaves_it_opaque() {
        let src = vec![0x40u8; 8 * 8 * 4];
        let (out, w, h) = upscale(src, 8, 8, Method::Xbrz, 3);
        assert_eq!((w, h), (24, 24));
        assert_eq!(out.len(), 24 * 24 * 4);
        assert!(out.as_chunks::<4>().0.iter().all(|px| px[3] == 0xFF));
    }
}
