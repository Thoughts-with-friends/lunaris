//! xBRZ: the edge-aware pixel-art upscaler for the DS's 2D picture.
//!
//! The same feature lunaris's own front end has (`gui/common/src/upscale.rs`),
//! so the two can be compared on equal terms. It is the only setting that can
//! improve a *2D* layer: those are drawn from tiles at 256x192 whatever the
//! renderer's internal resolution is.
//!
//! # Where it runs
//!
//! Always on the CPU, at 256x192 per screen, by [`upscale`]:
//!
//! * software renderer — on the framebuffers, inside `ui::layout::to_image`;
//! * OpenGL renderer — on the 2D content read back off the GPU
//!   (`ui::screen::filter_gl_2d`); the 3D keeps the GPU's own pixels.
//!
//! The factor (1 to 6) is one xBRZ pass; 1 means "off".

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
    let factor = usize::from(clamp_factor(factor));
    if method == Method::None || factor == 1 {
        return (rgba, width, height);
    }

    // xBRZ blends alpha at edges; the DS's pixels are opaque, so forcing that
    // first stops the filter inventing translucent fringes.
    let mut buf = rgba;
    for pixel in buf.as_chunks_mut::<4>().0 {
        pixel[3] = 0xFF;
    }
    buf = xbrz::scale_rgba(&buf, width, height, factor);
    // Its border interpolation is not alpha-aware either.
    for pixel in buf.as_chunks_mut::<4>().0 {
        pixel[3] = 0xFF;
    }
    (buf, width * factor, height * factor)
}

#[cfg(test)]
mod tests {
    use super::{MAX_FACTOR, MIN_FACTOR, Method, clamp_factor, upscale};

    /// Every offered factor scales the picture by exactly that much.
    #[test]
    fn every_factor_scales_by_exactly_itself() {
        for factor in MIN_FACTOR..=MAX_FACTOR {
            let (out, w, h) = upscale(vec![0x40u8; 8 * 4 * 4], 8, 4, Method::Xbrz, factor);
            let f = usize::from(factor);
            assert_eq!((w, h), (8 * f, 4 * f), "{factor}x");
            assert_eq!(out.len(), w * h * 4);
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
