// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-scanline 2D engine renderer: layer compositing.
//!
//! Every layer is first rendered into its own line buffer, then the layers
//! are composited per pixel by priority with windows and BLDCNT color
//! effects, the way melonDS `GPU2D_Soft` does it:
//!
//! ```text
//!  bg.rs:     BG0..BG3 (text / affine / extended / large bitmap / 3D) ─┐
//!  obj.rs:    OBJ (normal / affine / bitmap, + OBJ window)  ───────────┼─> composite ─> master brightness
//!  window.rs: windows (WIN0, WIN1, OBJWIN, outside)  ──────────────────┘   (top two layers, BLDCNT)
//! ```
//!
//! See GBATEK "DS Video BG Modes / Control", "DS Video OBJs" and "LCD I/O
//! Color Special Effects".
//!
//! ## Per-pixel compositing
//!
//! ```text
//!   for each x of the line:
//!     m = window mask (window.rs)
//!     walk priority 0..3; at each priority check OBJ, then BG0, BG1, BG2, BG3;
//!     keep the first two opaque layers that m allows:
//!         top    = first hit    (backdrop if none)
//!         second = second hit   (backdrop if none)
//!
//!     m bit 5 clear, or top not a BLDCNT 1st target -> output top
//!     otherwise by BLDCNT bits 6-7:
//!         1 alpha     top*EVA/16 + second*EVB/16   (if second is a 2nd target)
//!         2 brighten  top + (31 - top) * EVY/16
//!         3 darken    top - top * EVY/16
//!     always blended with second (if it is a 2nd target), whatever BLDCNT says:
//!         semi-transparent OBJ  -> EVA / EVB
//!         bitmap OBJ            -> its own alpha (attr2 bits 12-15)
//!         3D pixel on BG0       -> the 3D pixel's own 5-bit alpha
//!
//!   example, alpha blending with EVA = EVB = 8 (50% / 50%):
//!       top    red   (31, 0, 0)  \
//!                                 >  (31*8 + 0*8)/16 = 15 ...  -> (15, 0, 15) purple
//!       second blue  (0, 0, 31)  /
//!
//!   example, darken with EVY = 8:  (31, 31, 31) white -> (16, 16, 16) gray
//! ```
//!
//! GBATEK: [LCD I/O Color Special Effects](https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects)
use ds_mem_addr::{PIXELS_PER_LINE, SCANLINES};

use super::mosaic;
use crate::{Gpu, display::hires::hd_3d_info};

pub(super) const W: usize = PIXELS_PER_LINE;

/// Layer pixel flags (low 15 bits hold the BGR555 color).
pub(super) const OPAQUE: u32 = 1 << 31;
/// OBJ pixel is semi-transparent (OBJ mode 1).
pub(super) const OBJ_SEMI: u32 = 1 << 30;
/// OBJ pixel comes from a bitmap OBJ; bits 20-23 hold its alpha.
pub(super) const OBJ_BITMAP: u32 = 1 << 29;
/// BG0 pixel comes from the 3D engine.
pub(super) const BG_3D: u32 = 1 << 28;
/// OBJ pixel comes from an OBJ with mosaic on (attribute 0 bit 12).
pub(super) const OBJ_MOSAIC: u32 = 1 << 27;

/// Line buffers for one scanline of one engine.
pub(super) struct LineLayers {
    pub(super) bg: [[u32; W]; 4],
    /// OBJ color/flags; bits 16-17 hold the OBJ priority.
    pub(super) obj: [u32; W],
    pub(super) obj_window: [bool; W],
}

/// BGR555 -> 0xFFRRGGBB with 5-to-8-bit expansion.
#[inline]
pub(crate) const fn rgb555_to_888(c: u16) -> u32 {
    // LCD Color Palettes: https://problemkaputt.de/gbatek.htm#lcdcolorpalettes
    let r = (c & 0x1F) as u32; // red (bits 0-4)
    let g = ((c >> 5) & 0x1F) as u32; // green (bits 5-9)
    let b = ((c >> 10) & 0x1F) as u32; // blue (bits 10-14)
    0xFF00_0000 // opaque alpha
        | (((r << 3) | (r >> 2)) << 16) // 5-bit red widened to 8 bits, placed in bits 16-23
        | (((g << 3) | (g >> 2)) << 8)
        | ((b << 3) | (b >> 2))
}

/// BLDALPHA blend `(a * eva + b * evb) / 16` per channel, saturated.
#[inline]
fn blend(a: u16, b: u16, eva: u32, evb: u32) -> u16 {
    let ch = |shift: u16| -> u16 {
        // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
        let ca = u32::from((a >> shift) & 0x1F); // 5-bit color component
        let cb = u32::from((b >> shift) & 0x1F); // 5-bit color component
        (((ca * eva + cb * evb) >> 4).min(31) as u16) << shift
    };
    ch(0) | ch(5) | ch(10)
}

/// Blend with 5-bit weights (`eva + evb == 32`), used by 3D pixels.
#[inline]
pub(crate) fn blend32(a: u16, b: u16, eva: u32, evb: u32) -> u16 {
    let ch = |shift: u16| -> u16 {
        let ca = u32::from((a >> shift) & 0x1F); // 5-bit color component
        let cb = u32::from((b >> shift) & 0x1F); // 5-bit color component
        (((ca * eva + cb * evb) >> 5).min(31) as u16) << shift
    };
    ch(0) | ch(5) | ch(10)
}

/// BLDY brightness increase `c + (31 - c) * evy / 16`.
#[inline]
pub(crate) fn brighten(c: u16, evy: u32) -> u16 {
    let ch = |shift: u16| -> u16 {
        let v = u32::from((c >> shift) & 0x1F); // 5-bit color component
        ((v + (((31 - v) * evy) >> 4)) as u16) << shift
    };
    ch(0) | ch(5) | ch(10)
}

/// BLDY brightness decrease `c - c * evy / 16`.
#[inline]
pub(crate) fn darken(c: u16, evy: u32) -> u16 {
    let ch = |shift: u16| -> u16 {
        let v = u32::from((c >> shift) & 0x1F); // 5-bit color component
        ((v - ((v * evy) >> 4)) as u16) << shift
    };
    ch(0) | ch(5) | ch(10)
}

impl Gpu {
    /// OBJ palette entry `index` of engine A/B (standard palette).
    pub(crate) fn obj_palette_color(&self, a: bool, index: u32) -> u16 {
        let pal = if a { &self.palette_upper } else { &self.palette_lower };
        // LCD Color Palettes: https://problemkaputt.de/gbatek.htm#lcdcolorpalettes
        let i = 0x200 + ((index as usize * 2) & 0x1FF); // OBJ palette starts at offset 200h
        u16::from_le_bytes([pal[i], pal[i + 1]])
    }

    /// BG palette entry `index` of engine A/B (0 = backdrop).
    pub(crate) fn bg_palette_color(&self, a: bool, index: u32) -> u16 {
        let pal = if a { &self.palette_upper } else { &self.palette_lower };
        let i = (index as usize * 2) & 0x1FF; // BG palette at offset 0-1FFh
        u16::from_le_bytes([pal[i], pal[i + 1]])
    }

    /// Engine A (upper field) or B.
    pub(crate) fn engine(&self, a: bool) -> &crate::engine_2d::Gpu2DEngine {
        if a { &self.engine_upper } else { &self.engine_lower }
    }

    /// Mutable engine A or B.
    fn engine_mut(&mut self, a: bool) -> &mut crate::engine_2d::Gpu2DEngine {
        if a { &mut self.engine_upper } else { &mut self.engine_lower }
    }

    /// Renders and composites the current scanline of one engine into its
    /// `framebuffer` (pre-display-mode) row.
    ///
    /// ```text
    /// from: draw_scanline_for_engine() step 1
    ///   |
    ///   v
    /// compose_scanline(engine)
    ///   1. per enabled BG, by BG mode (DISPCNT bits 0-2):
    ///        BG0 + 3D on (engine A) -> render_bg_3d   (3D raster line)
    ///        text / affine / extended / large -> render_bg_* (bg.rs)
    ///   2. OBJ enabled -> render_objs()                (obj.rs)
    ///   3. composite()   window masks (window.rs), top two layers by
    ///                    priority, BLDCNT alpha / brighten / darken
    ///   4. advance_affine_refs()   BG2/BG3 reference point += PB/PD
    ///   |
    ///   v
    /// to:   draw_scanline_for_engine() applies the display mode
    /// ```
    pub fn compose_scanline(&mut self, a: bool) {
        let line = self.get_vcount() as usize;
        if line >= SCANLINES {
            return;
        }

        let mut layers = LineLayers { bg: [[0; W]; 4], obj: [0; W], obj_window: [false; W] };

        self.engine_mut(a).mosaic_line_start(line);
        let dispcnt = self.engine(a).dispcnt;
        let mode = dispcnt.bg_mode;
        let shown =
            [dispcnt.display_bg0, dispcnt.display_bg1, dispcnt.display_bg2, dispcnt.display_bg3];

        for (index, &on) in shown.iter().enumerate() {
            if !on {
                continue;
            }
            match (index, mode) {
                (0, _) if a && dispcnt.bg_3d => self.render_bg_3d(&mut layers.bg[0], line),
                (0 | 1, _) => self.render_bg_text(a, index, line, &mut layers.bg[index]),
                (2, 0 | 1 | 3) => self.render_bg_text(a, 2, line, &mut layers.bg[2]),
                (2, 2 | 4) => self.render_bg_affine(a, 2, &mut layers.bg[2]),
                (2, 5) => self.render_bg_extended(a, 2, &mut layers.bg[2]),
                (2, 6) if a => self.render_bg_large(&mut layers.bg[2]),
                (3, 0) => self.render_bg_text(a, 3, line, &mut layers.bg[3]),
                (3, 1 | 2) => self.render_bg_affine(a, 3, &mut layers.bg[3]),
                (3, 3..=5) => self.render_bg_extended(a, 3, &mut layers.bg[3]),
                _ => {}
            }
            // The 3D layer has no mosaic.
            let is_3d = index == 0 && a && dispcnt.bg_3d;
            if !is_3d && self.engine(a).bg_mosaic(index) {
                mosaic::bg_mosaic_x(&mut layers.bg[index], self.engine(a).bg_mosaic_size().0);
            }
        }

        if dispcnt.display_obj || dispcnt.obj_win_display {
            self.render_objs(a, line, &mut layers);
            mosaic::obj_mosaic_x(&mut layers.obj, self.engine(a).obj_mosaic_size().0);
        }

        self.composite(a, line, &layers);
        self.advance_affine_refs(a);
        self.engine_mut(a).mosaic_line_end();
    }

    /// Steps the internal BG2/BG3 affine reference points by PB/PD after
    /// each visible line (they are reloaded from BGxX/BGxY at VBlank).
    fn advance_affine_refs(&mut self, a: bool) {
        let e = self.engine_mut(a);
        e.bg2x_internal = e.bg2x_internal.wrapping_add(i32::from(e.bg2p[1] as i16));
        e.bg2y_internal = e.bg2y_internal.wrapping_add(i32::from(e.bg2p[3] as i16));
        e.bg3x_internal = e.bg3x_internal.wrapping_add(i32::from(e.bg3p[1] as i16));
        e.bg3y_internal = e.bg3y_internal.wrapping_add(i32::from(e.bg3p[3] as i16));
    }

    /// Picks the top two visible layers per pixel and applies BLDCNT.
    fn composite(&mut self, a: bool, line: usize, layers: &LineLayers) {
        let masks = self.window_masks(a, line, layers);
        let backdrop = self.bg_palette_color(a, 0) & 0x7FFF; // backdrop = BG palette color 0
        let e = self.engine(a);
        let bgcnt = e.bgcnt;
        let bld = e.bldcnt;
        // LCD I/O Color Special Effects: https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects
        let eva = u32::from(e.bldalpha & 0x1F).min(16); // BLDALPHA EVA (bits 0-4), capped at 16
        let evb = u32::from((e.bldalpha >> 8) & 0x1F).min(16); // BLDALPHA EVB (bits 8-12), capped at 16
        let evy = u32::from(e.bldy & 0x1F).min(16); // BLDY EVY (bits 0-4), capped at 16
        // BLDCNT targets per layer id (0-3 BG, 4 OBJ, 5 backdrop).
        let firsts = [
            bld.bg_first_target_pix[0],
            bld.bg_first_target_pix[1],
            bld.bg_first_target_pix[2],
            bld.bg_first_target_pix[3],
            bld.obj_first_target_pix,
            bld.bd_first_target_pix,
        ];
        let seconds = [
            bld.bg_second_target_pix[0],
            bld.bg_second_target_pix[1],
            bld.bg_second_target_pix[2],
            bld.bg_second_target_pix[3],
            bld.obj_second_target_pix,
            bld.bd_second_target_pix,
        ];
        let first_target = |layer: usize| firsts[layer];
        let second_target = |layer: usize| seconds[layer];
        let effect = bld.effect;

        // Displayed BGs grouped by priority (lower index first), so each
        // pixel checks only the layers that exist instead of 4 x 4 pairs.
        let d = e.dispcnt;
        let shown = [d.display_bg0, d.display_bg1, d.display_bg2, d.display_bg3];
        let mut groups = [[0usize; 4]; 4];
        let mut group_len = [0usize; 4];
        for bg in 0..4 {
            if shown[bg] {
                let p = usize::from(bgcnt[bg] & 3);
                groups[p][group_len[p]] = bg;
                group_len[p] += 1;
            }
        }

        let mut row = [0u32; W];
        // Engine A: how each 3D pixel was composited (display::hires).
        let mut hd = [0u32; W];
        for x in 0..W {
            let mask = masks[x];
            // (layer id, pixel); layer 5 = backdrop.
            let mut found = [(5usize, u32::from(backdrop)); 2];
            let mut count = 0;
            let obj = layers.obj[x];
            // window mask bit 4: OBJ enabled
            let obj_prio =
                if obj & OPAQUE != 0 && mask & 0x10 != 0 { ((obj >> 16) & 3) as usize } else { 4 };
            'search: for prio in 0..4 {
                if obj_prio == prio {
                    found[count] = (4, obj);
                    count += 1;
                    if count == 2 {
                        break 'search;
                    }
                }
                for &bg in &groups[prio][..group_len[prio]] {
                    let px = layers.bg[bg][x];
                    if px & OPAQUE != 0 && mask & (1 << bg) != 0 {
                        found[count] = (bg, px);
                        count += 1;
                        if count == 2 {
                            break 'search;
                        }
                    }
                }
            }

            let (top_layer, top) = found[0];
            let (second_layer, second) = found[1];
            let top_color = (top & 0x7FFF) as u16; // BGR555 color
            let second_color = (second & 0x7FFF) as u16; // BGR555 color
            let effects = mask & 0x20 != 0; // window mask bit 5: color special effects enabled

            // Fast path: no special effect possible for this pixel.
            let color = if !effects || (effect == 0 && top & (OBJ_SEMI | OBJ_BITMAP | BG_3D) == 0) {
                top_color
            } else if top_layer == 4
                && top & (OBJ_SEMI | OBJ_BITMAP) != 0
                && second_target(second_layer)
            {
                // Semi-transparent / bitmap OBJs blend regardless of BLDCNT mode.
                if top & OBJ_BITMAP != 0 {
                    let alpha = ((top >> 20) & 0xF) + 1; // bitmap OBJ alpha (bits 20-23) + 1
                    blend(top_color, second_color, alpha, 16 - alpha)
                } else {
                    blend(top_color, second_color, eva, evb)
                }
            } else if top_layer == 0 && top & BG_3D != 0 && second_target(second_layer) {
                // 3D pixels blend with their own alpha regardless of the
                // BLDCNT mode (GBATEK "3D BG0 Blending").
                let eva = ((top >> 16) & 0x1F) + 1;
                blend32(top_color, second_color, eva, 32 - eva)
            } else if top_layer == 0 && top & BG_3D != 0 && bld.effect == 1 {
                // Alpha blending without a second target leaves 3D as is.
                top_color
            } else if !first_target(top_layer) {
                top_color
            } else {
                match bld.effect {
                    1 if second_target(second_layer) => blend(top_color, second_color, eva, evb),
                    2 => brighten(top_color, evy),
                    3 => darken(top_color, evy),
                    _ => top_color,
                }
            };
            row[x] = rgb555_to_888(color);

            if a && top_layer == 0 && top & BG_3D != 0 {
                let blended = effects && second_target(second_layer);
                let effect = if effects && !blended && bld.effect != 1 && first_target(0) {
                    u32::from(bld.effect)
                } else {
                    0
                };
                hd[x] = hd_3d_info(second_color, blended, effect, evy);
            }
        }

        let start = line * W;
        self.engine_mut(a).framebuffer[start..start + W].copy_from_slice(&row);
        if a {
            self.hd_3d[start..start + W].copy_from_slice(&hd);
        }
    }
}
