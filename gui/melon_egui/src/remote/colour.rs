//! The one place the framebuffer's channel order is decided.
//!
//! Everything else in [`crate::remote`] moves 16-bit pixels around without
//! caring what the bits mean. Keeping the two conversions together — and
//! nowhere else — is deliberate: an independently written channel order that
//! swapped red and blue would look plausible until somebody noticed the skin
//! tones were wrong.

/// Pack one framebuffer pixel into RGB565.
///
/// The core hands pixels over as `0x00RRGGBB` in a `u32` — the same order
/// `crate::app::to_image` reads them in, which is where this shift pattern
/// comes from.
///
/// 565 rather than the framebuffer's 888 halves the bytes and costs almost
/// nothing: the DS's own output is 6 bits per channel (GBATEK, "DS Video BG
/// Modes"), so green is exact and red and blue lose one bit each.
#[must_use]
pub const fn to_565(pixel: u32) -> u16 {
    let r = ((pixel >> 16) & 0xFF) as u16;
    let g = ((pixel >> 8) & 0xFF) as u16;
    let b = (pixel & 0xFF) as u16;
    ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3)
}

/// Unpack RGB565 back into the framebuffer's `0x00RRGGBB`.
///
/// The low bits are filled from the high ones rather than with zeros, so that
/// full white stays full white — a plain shift would turn `0xFF` into `0xF8`
/// and tint every bright area very slightly dark.
#[must_use]
pub const fn from_565(pixel: u16) -> u32 {
    let r = ((pixel >> 11) & 0x1F) as u32;
    let g = ((pixel >> 5) & 0x3F) as u32;
    let b = (pixel & 0x1F) as u32;
    let r = (r << 3) | (r >> 2);
    let g = (g << 2) | (g >> 4);
    let b = (b << 3) | (b >> 2);
    (r << 16) | (g << 8) | b
}

#[cfg(test)]
mod tests {
    use super::{from_565, to_565};

    #[test]
    fn colour_survives_the_round_trip_within_565() {
        for pixel in [0x00_00_00_00, 0x00_FF_FF_FF, 0x00_FF_00_00, 0x00_00_FF_00, 0x00_00_00_FF] {
            let back = from_565(to_565(pixel));
            for shift in [16, 8, 0] {
                let (before, after) = ((pixel >> shift) & 0xFF, (back >> shift) & 0xFF);
                assert!(
                    before.abs_diff(after) <= 4,
                    "channel at {shift} moved from {before} to {after}"
                );
            }
        }
        // The two that must be exact, or every bright area is tinted.
        assert_eq!(from_565(to_565(0x00_FF_FF_FF)), 0x00_FF_FF_FF);
        assert_eq!(from_565(to_565(0)), 0);
    }

    /// Red must not come back as blue. Checked against a pixel that is only
    /// red, because a swapped pair is otherwise invisible in grey test data.
    #[test]
    fn the_channels_stay_in_their_lanes() {
        let red = from_565(to_565(0x00_F8_00_00));
        assert!(red >> 16 > 0xF0, "red did not come back red: {red:#08x}");
        assert_eq!(red & 0xFF, 0, "blue appeared from nowhere: {red:#08x}");
    }
}
