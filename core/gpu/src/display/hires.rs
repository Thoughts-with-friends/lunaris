// SPDX-License-Identifier: GPL-3.0-or-later
//! Screen images above the native resolution: the 2D picture upscaled
//! (nearest neighbour or xBRZ), with engine A's 3D pixels replaced by the
//! high-resolution 3D frame.
//!
//! ```text
//!  compose.rs composite()  (engine A, per native pixel)
//!     Hd3dInfo: top layer is 3D? + the layer below + how it was blended
//!  output.rs draw_scanline_for_engine()  (engine A, per line)
//!     HdLine: display mode, MASTER_BRIGHT, BG0HOFS
//!       |
//!       v
//!  Gpu::hd_frame() -> HdFrame: all of the above plus both native screens
//!     and the 3D snapshot, owned (can be sent to another thread)
//!  HdFrame::screen(renderer, upper, scale, xbrz)   (Gpu::screen_hd = both)
//!     1. native front framebuffer -> xBRZ / nearest  (scale x scale)
//!     2. engine A's screen and the snapshot rendered at that scale:
//!          every 3D pixel's block re-blended from the hi-res 3D
//!          (3D alpha blending / BLDY / master brightness as natively)
//! ```
//!
//! Display capture and everything the game can read stay native.
use std::sync::Arc;

use ds_mem_addr::{PIXELS_PER_LINE, SCANLINES};
use rayon::prelude::*;

use super::super::engine_2d::compose::{blend32, brighten, darken, rgb555_to_888};
use crate::{
    Gpu,
    engine_3d::raster::{
        Raster,
        hires::{Renderer, Snapshot},
    },
};

/// [`Hd3dInfo`] bit: the visible pixel is 3D (BG0 of engine A on top).
pub(crate) const HD_3D: u32 = 1 << 31;
/// The 3D pixel was alpha-blended with the layer below (bits 0-14).
pub(crate) const HD_BLEND: u32 = 1 << 30;
/// Bits 16-17: BLDCNT effect applied to the 3D pixel (2 brighten,
/// 3 darken); bits 18-22: EVY.
const HD_EFFECT_SHIFT: u32 = 16;
const HD_EVY_SHIFT: u32 = 18;

/// Encodes how a 3D pixel was composited (see the bit constants).
pub(crate) const fn hd_3d_info(below: u16, blend: bool, effect: u32, evy: u32) -> u32 {
    HD_3D
        | (blend as u32) << 30 // bit 30: 3D pixel takes part in alpha blending
        | (below as u32 & 0x7FFF) // BGR555 color of the 2D layer below
        | (effect & 3) << HD_EFFECT_SHIFT
        | (evy & 0x1F) << HD_EVY_SHIFT // BLDY EVY (bits 0-4)
}

/// Engine A state of one line that the hi-res pass needs.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default)]
pub(crate) struct HdLine {
    /// DISPCNT display mode 1 (graphics) on this line.
    pub(crate) graphics: bool,
    pub(crate) master_bright: u16,
    pub(crate) bg0_hofs: u16,
}

/// MASTER_BRIGHT applied to a 0xAARRGGBB pixel (same math as the native
/// output in `output.rs`).
fn master_bright(px: u32, reg: u16) -> u32 {
    if !matches!(reg >> 14, 1 | 2) {
        return px;
    }
    // DS Video Stuff: https://problemkaputt.de/gbatek.htm#dsvideostuff
    let factor = f32::from((reg & 0x1F).min(16)) / 16.0; // MASTER_BRIGHT factor (bits 0-4), capped at 16
    let ch = |shift: u32| ((px >> shift) & 0xFF) as f32; // 8-bit RGB channel
    let (r, g, b) = (ch(16), ch(8), ch(0));
    let (r, g, b) = match reg >> 14 {
        1 => (r + (255.0 - r) * factor, g + (255.0 - g) * factor, b + (255.0 - b) * factor),
        2 => (r - r * factor, g - g * factor, b - b * factor),
        _ => return px,
    };
    0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32) // opaque ARGB8888
}

/// Horizontal bands xBRZ works on in parallel.
const XBRZ_BANDS: usize = 8;
/// Rows of real neighbours given to each band above and below (xBRZ looks
/// two rows away; more keeps the bands' edges identical to a whole-image
/// pass).
const XBRZ_OVERLAP: usize = 4;

/// 0xAARRGGBB -> `scale`x upscaled 0xFFRRGGBB with xBRZ, in parallel bands.
fn xbrz(src: &[u32], scale: usize) -> Vec<u32> {
    let w = PIXELS_PER_LINE;
    let rows = SCANLINES.div_ceil(XBRZ_BANDS);
    let band = |y0: usize| -> Vec<u32> {
        let y1 = (y0 + rows).min(SCANLINES);
        let (p0, p1) = (y0.saturating_sub(XBRZ_OVERLAP), (y1 + XBRZ_OVERLAP).min(SCANLINES));
        let rgba: Vec<u8> = src[p0 * w..p1 * w]
            .iter()
            .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, 0xFF]) // RGBA bytes with opaque alpha
            .collect();
        let out = xbrz::scale_rgba(&rgba, w, p1 - p0, scale);
        let row_bytes = w * scale * 4;
        let keep = &out[(y0 - p0) * scale * row_bytes..(y1 - p0) * scale * row_bytes];
        keep.as_chunks::<4>()
            .0
            .iter()
            .map(|c| 0xFF00_0000 | u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2])) // opaque ARGB8888
            .collect()
    };
    crate::threads::run(|| {
        (0..XBRZ_BANDS).into_par_iter().map(|i| band(i * rows)).collect::<Vec<_>>()
    })
    .concat()
}

/// 0xAARRGGBB -> `scale`x upscaled with nearest neighbour.
fn nearest(src: &[u32], scale: usize, out: &mut Vec<u32>) {
    let w = PIXELS_PER_LINE * scale;
    out.clear();
    out.reserve(w * SCANLINES * scale);
    for row in src.as_chunks::<PIXELS_PER_LINE>().0 {
        let start = out.len();
        for &p in row {
            out.extend(std::iter::repeat_n(p, scale));
        }
        for _ in 1..scale {
            out.extend_from_within(start..start + w);
        }
    }
}

/// Everything needed to draw both screens above the native resolution,
/// taken from the GPU after a frame (see [`Gpu::hd_frame`]).
#[derive(Debug, Clone)]
pub struct HdFrame {
    upper: Vec<u32>,
    lower: Vec<u32>,
    /// Engine A (the 3D) is on the upper screen.
    engine_a_upper: bool,
    bg_3d: bool,
    info: Vec<u32>,
    lines: [HdLine; SCANLINES],
    snapshot: Option<Arc<Snapshot>>,
    /// Internal resolution of the snapshot's 3D.
    scale_3d: usize,
}

impl Gpu {
    /// Copies what [`HdFrame::screen`] needs from the last frame.
    pub fn hd_frame(&mut self) -> HdFrame {
        let mut upper = vec![0; PIXELS_PER_LINE * SCANLINES];
        let mut lower = vec![0; PIXELS_PER_LINE * SCANLINES];
        self.get_upper_frame(&mut upper);
        self.get_lower_frame(&mut lower);
        let hires = &self.engine_3d.hires;
        HdFrame {
            upper,
            lower,
            engine_a_upper: self.display_swapped(),
            bg_3d: self.engine_upper.dispcnt.bg_3d,
            info: self.hd_3d.clone(),
            lines: self.hd_lines,
            snapshot: hires.snapshot(),
            scale_3d: hires.scale(),
        }
    }

    /// The upper (`upper`) or lower screen at `scale` x 256 x 192, with the
    /// 2D graphics xBRZ-filtered when `xbrz` is set and 3D at the internal
    /// resolution when it matches `scale` (see [`Gpu::set_render_scale`]).
    /// Returns the width; `out` holds the 0xAARRGGBB pixels.
    pub fn screen_hd(
        &mut self,
        upper: bool,
        scale: usize,
        xbrz_filter: bool,
        out: &mut Vec<u32>,
    ) -> usize {
        let frame = self.hd_frame();
        frame.screen(&mut self.engine_3d.hires.renderer, upper, scale, xbrz_filter, out)
    }
}

impl HdFrame {
    /// One screen as [`Gpu::screen_hd`] draws it; `renderer` keeps the
    /// scaled 3D between calls.
    pub fn screen(
        &self,
        renderer: &mut Renderer,
        upper: bool,
        scale: usize,
        xbrz_filter: bool,
        out: &mut Vec<u32>,
    ) -> usize {
        let scale = scale.clamp(1, 6);
        let native = if upper { &self.upper } else { &self.lower };
        if scale == 1 {
            out.clone_from(native);
            return PIXELS_PER_LINE;
        }
        if xbrz_filter {
            *out = xbrz(native, scale);
        } else {
            nearest(native, scale, out);
        }
        if upper == self.engine_a_upper
            && self.bg_3d
            && self.scale_3d == scale
            && let Some(snap) = &self.snapshot
        {
            self.overlay_hires_3d(renderer.render(snap, scale), scale, out);
        }
        PIXELS_PER_LINE * scale
    }

    /// Replaces the 3D pixels of `out` (engine A's screen, upscaled) with
    /// the high-resolution 3D frame `hi`.
    fn overlay_hires_3d(&self, hi: &Raster, scale: usize, out: &mut [u32]) {
        let (info, lines) = (&self.info, &self.lines);
        let w = PIXELS_PER_LINE * scale;
        // One native line (scale rows of `out`) per task.
        crate::threads::run(|| {
            out.par_chunks_mut(w * scale)
                .zip(lines.par_iter())
                .enumerate()
                .filter(|(_, (_, line))| line.graphics)
                .for_each(|(y, (block, line))| {
                    // LCD I/O BG Scrolling: https://problemkaputt.de/gbatek.htm#lcdiobgscrolling
                    let xpos = usize::from(line.bg0_hofs & 0x1FF); // BG0HOFS X offset (bits 0-8)
                    for x in 0..PIXELS_PER_LINE {
                        let i = info[y * PIXELS_PER_LINE + x];
                        if i & HD_3D == 0 {
                            continue;
                        }
                        // Same scrolling as render_bg_3d.
                        let sx = x + xpos;
                        let sx = if xpos & 0x100 != 0 {
                            // bit 8: negative scroll
                            sx.wrapping_sub(512)
                        } else {
                            sx
                        };
                        if sx >= PIXELS_PER_LINE {
                            continue;
                        }
                        for j in 0..scale {
                            let src = &hi.line(y * scale + j)[sx * scale..(sx + 1) * scale];
                            let dst = &mut block[j * w + x * scale..j * w + (x + 1) * scale];
                            for (d, &c) in dst.iter_mut().zip(src) {
                                let px = rgb555_to_888(shade_3d(c, i));
                                *d = master_bright(px, line.master_bright);
                            }
                        }
                    }
                })
        });
    }
}

/// One hi-res 3D pixel composited the way its native pixel was.
fn shade_3d(c: u32, info: u32) -> u16 {
    let below = (info & 0x7FFF) as u16; // BGR555 color below the 3D pixel
    let alpha = (c >> 24) & 0x1F; // 3D pixel alpha (bits 24-28)
    if alpha == 0 {
        return below;
    }
    let r = (c >> 1) & 0x1F; // 6-bit red to 5 bits
    let g = (c >> 9) & 0x1F; // 6-bit green to 5 bits
    let b = (c >> 17) & 0x1F; // 6-bit blue to 5 bits
    let color = (r | (g << 5) | (b << 10)) as u16; // pack BGR555
    if info & HD_BLEND != 0 {
        let eva = alpha + 1;
        return blend32(color, below, eva, 32 - eva);
    }
    let evy = (info >> HD_EVY_SHIFT) & 0x1F; // BLDY EVY (bits 0-4)
    match (info >> HD_EFFECT_SHIFT) & 3 {
        2 => brighten(color, evy),
        3 => darken(color, evy),
        _ => color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_scales_rows_and_columns() {
        let mut src = vec![0; PIXELS_PER_LINE * SCANLINES];
        src[1] = 7;
        src[PIXELS_PER_LINE] = 9;
        let mut out = Vec::new();
        nearest(&src, 2, &mut out);
        let w = PIXELS_PER_LINE * 2;
        assert_eq!(out.len(), w * SCANLINES * 2);
        assert_eq!(&out[..4], &[0, 0, 7, 7]);
        assert_eq!(&out[w..w + 4], &[0, 0, 7, 7]);
        assert_eq!(out[2 * w], 9);
        assert_eq!(out[3 * w + 1], 9);
    }

    #[test]
    fn xbrz_keeps_flat_areas() {
        let src = vec![0xFF12_3456; PIXELS_PER_LINE * SCANLINES]; // opaque test color
        let out = xbrz(&src, 3);
        assert_eq!(out.len(), PIXELS_PER_LINE * SCANLINES * 9);
        // Away from the image border the color is untouched.
        assert_eq!(out[100 * PIXELS_PER_LINE * 3 + 300], 0xFF12_3456);
    }

    #[test]
    fn xbrz_bands_match_a_whole_image_pass() {
        // A busy pattern: diagonals and blocks crossing the band borders.
        let src: Vec<u32> = (0..PIXELS_PER_LINE * SCANLINES)
            .map(|i| {
                let (x, y) = (i % PIXELS_PER_LINE, i / PIXELS_PER_LINE);
                if (x + y) % 7 < 3 || (x / 5 + y / 3) % 4 == 0 {
                    0xFFFF_8040 // opaque test color
                } else {
                    0xFF10_2030 // opaque test color
                }
            })
            .collect();
        let rgba: Vec<u8> = src
            .iter()
            .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, 0xFF]) // RGBA bytes with opaque alpha
            .collect();
        let whole: Vec<u32> = xbrz::scale_rgba(&rgba, PIXELS_PER_LINE, SCANLINES, 4)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| 0xFF00_0000 | u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2])) // opaque ARGB8888
            .collect();
        assert!(xbrz(&src, 4) == whole);
    }

    #[test]
    fn shading_matches_native_rules() {
        // Opaque white 3D (6-bit 63 -> 5-bit 31), alpha 31.
        let white = 0x1F00_0000 | 0x3F | (0x3F << 8) | (0x3F << 16);
        assert_eq!(shade_3d(white, hd_3d_info(0, false, 0, 0)), 0x7FFF);
        // Transparent 3D shows the layer below.
        assert_eq!(shade_3d(0, hd_3d_info(0x1234, false, 0, 0)), 0x1234);
        // Darken by 16 -> black.
        assert_eq!(shade_3d(white, hd_3d_info(0, false, 3, 16)), 0);
        // Half alpha blends with black below.
        let half = (white & 0x00FF_FFFF) | (15 << 24);
        assert_eq!(shade_3d(half, hd_3d_info(0, true, 0, 0)) & 0x1F, 15); // 5-bit blended red
        assert_eq!(master_bright(0xFF00_0000, (2 << 14) | 16), 0xFF00_0000); // MASTER_BRIGHT mode 2 (down), factor 16: black stays black
        assert_eq!(master_bright(0xFF00_0000, (1 << 14) | 16), 0xFFFF_FFFF); // MASTER_BRIGHT mode 1 (up), factor 16: white
    }
}
