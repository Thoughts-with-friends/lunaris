// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-scanline 2D engine renderer.
//!
//! Every layer is first rendered into its own line buffer, then the layers
//! are composited per pixel by priority with windows and BLDCNT color
//! effects, the way melonDS `GPU2D_Soft` does it:
//!
//! ```text
//!  BG0..BG3 (text / affine / extended / large bitmap / 3D) ─┐
//!  OBJ (normal / affine / bitmap, + OBJ window)  ───────────┼─> composite ─> master brightness
//!  windows (WIN0, WIN1, OBJWIN, outside)  ──────────────────┘   (top two layers, BLDCNT)
//! ```
//!
//! See GBATEK "DS Video BG Modes / Control", "DS Video OBJs" and "LCD I/O
//! Color Special Effects".
use crate::gpu_root::Gpu;
use crate::vram::Region;
use lunaris_ds_mem_const::{PIXELS_PER_LINE, SCANLINES};

const W: usize = PIXELS_PER_LINE;

/// Layer pixel flags (low 15 bits hold the BGR555 color).
const OPAQUE: u32 = 1 << 31;
/// OBJ pixel is semi-transparent (OBJ mode 1).
const OBJ_SEMI: u32 = 1 << 30;
/// OBJ pixel comes from a bitmap OBJ; bits 20-23 hold its alpha.
const OBJ_BITMAP: u32 = 1 << 29;
/// BG0 pixel comes from the 3D engine.
const BG_3D: u32 = 1 << 28;

/// OBJ sizes in pixels: `[shape][size] = (width, height)`.
const OBJ_SIZES: [[(u32, u32); 4]; 4] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)],
    [(16, 8), (32, 8), (32, 16), (64, 32)],
    [(8, 16), (8, 32), (16, 32), (32, 64)],
    [(8, 8), (8, 8), (8, 8), (8, 8)], // prohibited shape
];

/// Line buffers for one scanline of one engine.
struct LineLayers {
    bg: [[u32; W]; 4],
    /// OBJ color/flags; bits 16-17 hold the OBJ priority.
    obj: [u32; W],
    obj_window: [bool; W],
}

#[inline]
const fn rgb555_to_888(c: u16) -> u32 {
    let r = (c & 0x1F) as u32;
    let g = ((c >> 5) & 0x1F) as u32;
    let b = ((c >> 10) & 0x1F) as u32;
    0xFF00_0000
        | (((r << 3) | (r >> 2)) << 16)
        | (((g << 3) | (g >> 2)) << 8)
        | ((b << 3) | (b >> 2))
}

#[inline]
const fn sign_extend_28(v: i32) -> i32 {
    (v << 4) >> 4
}

#[inline]
fn blend(a: u16, b: u16, eva: u32, evb: u32) -> u16 {
    let ch = |shift: u16| -> u16 {
        let ca = u32::from((a >> shift) & 0x1F);
        let cb = u32::from((b >> shift) & 0x1F);
        (((ca * eva + cb * evb) >> 4).min(31) as u16) << shift
    };
    ch(0) | ch(5) | ch(10)
}

#[inline]
fn brighten(c: u16, evy: u32) -> u16 {
    let ch = |shift: u16| -> u16 {
        let v = u32::from((c >> shift) & 0x1F);
        ((v + (((31 - v) * evy) >> 4)) as u16) << shift
    };
    ch(0) | ch(5) | ch(10)
}

#[inline]
fn darken(c: u16, evy: u32) -> u16 {
    let ch = |shift: u16| -> u16 {
        let v = u32::from((c >> shift) & 0x1F);
        ((v - ((v * evy) >> 4)) as u16) << shift
    };
    ch(0) | ch(5) | ch(10)
}

/// Whether `x` lies in the window span `[start, end)` (wrapping when
/// `start > end`, as the hardware does).
#[inline]
const fn in_span(x: u32, start: u32, end: u32) -> bool {
    if start <= end {
        x >= start && x < end
    } else {
        x >= start || x < end
    }
}

impl Gpu {
    fn engine(&self, a: bool) -> &crate::gpu_2d::Gpu2DEngine {
        if a {
            &self.engine_upper
        } else {
            &self.engine_lower
        }
    }

    fn engine_mut(&mut self, a: bool) -> &mut crate::gpu_2d::Gpu2DEngine {
        if a {
            &mut self.engine_upper
        } else {
            &mut self.engine_lower
        }
    }

    fn bg_palette_color(&self, a: bool, index: u32) -> u16 {
        let pal = if a {
            &self.palette_upper
        } else {
            &self.palette_lower
        };
        let i = (index as usize * 2) & 0x1FF;
        u16::from_le_bytes([pal[i], pal[i + 1]])
    }

    fn obj_palette_color(&self, a: bool, index: u32) -> u16 {
        let pal = if a {
            &self.palette_upper
        } else {
            &self.palette_lower
        };
        let i = 0x200 + ((index as usize * 2) & 0x1FF);
        u16::from_le_bytes([pal[i], pal[i + 1]])
    }

    /// Renders and composites the current scanline of one engine into its
    /// `framebuffer` (pre-display-mode) row.
    pub fn compose_scanline(&mut self, a: bool) {
        let line = self.get_vcount() as usize;
        if line >= SCANLINES {
            return;
        }

        let mut layers = LineLayers {
            bg: [[0; W]; 4],
            obj: [0; W],
            obj_window: [false; W],
        };

        let dispcnt = self.engine(a).dispcnt;
        let mode = dispcnt.bg_mode;
        let shown = [
            dispcnt.display_bg0,
            dispcnt.display_bg1,
            dispcnt.display_bg2,
            dispcnt.display_bg3,
        ];

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
        }

        if dispcnt.display_obj || dispcnt.obj_win_display {
            self.render_objs(a, line, &mut layers);
        }

        self.composite(a, line, &layers);
        self.advance_affine_refs(a);
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

    /// BG0 as the 3D layer: lets the 3D rasterizer draw into a cleared row
    /// and takes every pixel it touched.
    fn render_bg_3d(&mut self, out: &mut [u32; W], line: usize) {
        let start = line * W;
        self.engine_upper.framebuffer[start..start + W].fill(0);
        let priority = (self.engine_upper.bgcnt[0] & 3) as u8;
        self.render_scanline(true, priority);
        for (x, px) in out.iter_mut().enumerate() {
            let c = self.engine_upper.framebuffer[start + x];
            if c & 0xFF00_0000 != 0 {
                let r = (c >> 19) & 0x1F;
                let g = (c >> 11) & 0x1F;
                let b = (c >> 3) & 0x1F;
                *px = OPAQUE | BG_3D | (b << 10) | (g << 5) | r;
            }
        }
    }

    /// Text BG (GBATEK "BG Screen Data Formats", text mode).
    fn render_bg_text(&self, a: bool, index: usize, line: usize, out: &mut [u32; W]) {
        let region = if a { Region::BgA } else { Region::BgB };
        let e = self.engine(a);
        let bgcnt = e.bgcnt[index];
        let hofs = u32::from(e.bghofs[index]);
        let vofs = u32::from(e.bgvofs[index]);
        let (char_base, screen_base) = if a {
            (
                e.dispcnt.char_base as u32 * 0x1_0000,
                e.dispcnt.screen_base as u32 * 0x1_0000,
            )
        } else {
            (0, 0)
        };
        let char_base = char_base + u32::from((bgcnt >> 2) & 0xF) * 0x4000;
        let screen_base = screen_base + u32::from((bgcnt >> 8) & 0x1F) * 0x800;
        let color_256 = bgcnt & (1 << 7) != 0;
        let size = (bgcnt >> 14) & 0x3;
        let ext_palette = color_256 && e.dispcnt.bg_extended_palette;
        let ext_slot = if index < 2 && bgcnt & (1 << 13) != 0 {
            index + 2
        } else {
            index
        } as u32;
        let ext_region = if a {
            Region::BgExtPalA
        } else {
            Region::BgExtPalB
        };

        let width_mask = if size & 1 != 0 { 511 } else { 255 };
        let height_mask = if size & 2 != 0 { 511 } else { 255 };
        let y = (vofs + line as u32) & height_mask;
        let fine_y = y & 7;
        let mut row_base = screen_base + ((y >> 3) & 31) * 64;
        if y >= 256 {
            row_base += if size == 3 { 0x1000 } else { 0x800 };
        }

        let mut cached_column = u32::MAX;
        let mut entry = 0u16;
        for (px, out_px) in out.iter_mut().enumerate() {
            let x = (hofs + px as u32) & width_mask;
            let column = x >> 3;
            if column != cached_column {
                cached_column = column;
                let block = if x >= 256 { 0x800 } else { 0 };
                entry = self
                    .vram
                    .read_u16(region, row_base + block + (column & 31) * 2);
            }
            let tile = u32::from(entry & 0x3FF);
            let tx = if entry & (1 << 10) != 0 {
                7 - (x & 7)
            } else {
                x & 7
            };
            let ty = if entry & (1 << 11) != 0 {
                7 - fine_y
            } else {
                fine_y
            };
            let pal_bank = u32::from(entry >> 12);

            let color = if color_256 {
                let ci = u32::from(
                    self.vram
                        .read_u8(region, char_base + tile * 64 + ty * 8 + tx),
                );
                if ci == 0 {
                    continue;
                }
                if ext_palette {
                    self.vram
                        .read_u16(ext_region, ext_slot * 0x2000 + pal_bank * 512 + ci * 2)
                } else {
                    self.bg_palette_color(a, ci)
                }
            } else {
                let byte = self
                    .vram
                    .read_u8(region, char_base + tile * 32 + ty * 4 + tx / 2);
                let ci = u32::from(if tx & 1 != 0 { byte >> 4 } else { byte & 0xF });
                if ci == 0 {
                    continue;
                }
                self.bg_palette_color(a, pal_bank * 16 + ci)
            };
            *out_px = OPAQUE | u32::from(color & 0x7FFF);
        }
    }

    /// Current affine parameters of BG2/BG3: (ref x, ref y, pa, pc).
    fn affine_params(&self, a: bool, index: usize) -> (i32, i32, i32, i32) {
        let e = self.engine(a);
        if index == 2 {
            (
                sign_extend_28(e.bg2x_internal),
                sign_extend_28(e.bg2y_internal),
                i32::from(e.bg2p[0] as i16),
                i32::from(e.bg2p[2] as i16),
            )
        } else {
            (
                sign_extend_28(e.bg3x_internal),
                sign_extend_28(e.bg3y_internal),
                i32::from(e.bg3p[0] as i16),
                i32::from(e.bg3p[2] as i16),
            )
        }
    }

    /// Walks the affine texture coordinates of one line, calling `f(px, x, y)`
    /// for every pixel inside the `width` x `height` area (wrapping when
    /// `wrap`).
    fn affine_walk(
        &self,
        a: bool,
        index: usize,
        width: i32,
        height: i32,
        wrap: bool,
        mut f: impl FnMut(usize, u32, u32),
    ) {
        let (mut rx, mut ry, pa, pc) = self.affine_params(a, index);
        for px in 0..W {
            let (mut x, mut y) = (rx >> 8, ry >> 8);
            rx = rx.wrapping_add(pa);
            ry = ry.wrapping_add(pc);
            if wrap {
                x &= width - 1;
                y &= height - 1;
            } else if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            f(px, x as u32, y as u32);
        }
    }

    /// Rotation/scaling BG with 8-bit map entries and 8bpp tiles.
    fn render_bg_affine(&self, a: bool, index: usize, out: &mut [u32; W]) {
        let region = if a { Region::BgA } else { Region::BgB };
        let e = self.engine(a);
        let bgcnt = e.bgcnt[index];
        let (char_base, screen_base) = if a {
            (
                e.dispcnt.char_base as u32 * 0x1_0000,
                e.dispcnt.screen_base as u32 * 0x1_0000,
            )
        } else {
            (0, 0)
        };
        let char_base = char_base + u32::from((bgcnt >> 2) & 0xF) * 0x4000;
        let screen_base = screen_base + u32::from((bgcnt >> 8) & 0x1F) * 0x800;
        let size = 128 << ((bgcnt >> 14) & 3);
        let wrap = bgcnt & (1 << 13) != 0;
        let tiles_per_row = (size / 8) as u32;

        self.affine_walk(a, index, size, size, wrap, |px, x, y| {
            let tile = u32::from(
                self.vram
                    .read_u8(region, screen_base + (y / 8) * tiles_per_row + x / 8),
            );
            let ci = u32::from(
                self.vram
                    .read_u8(region, char_base + tile * 64 + (y & 7) * 8 + (x & 7)),
            );
            if ci != 0 {
                out[px] = OPAQUE | u32::from(self.bg_palette_color(a, ci) & 0x7FFF);
            }
        });
    }

    /// Extended BG: affine 16-bit map entries, 256-color or direct bitmap.
    fn render_bg_extended(&self, a: bool, index: usize, out: &mut [u32; W]) {
        let region = if a { Region::BgA } else { Region::BgB };
        let e = self.engine(a);
        let bgcnt = e.bgcnt[index];
        let wrap = bgcnt & (1 << 13) != 0;
        let size_bits = (bgcnt >> 14) & 3;

        if bgcnt & (1 << 7) == 0 {
            // Affine tiled BG with text-style 16-bit entries, 8bpp tiles.
            let (char_base, screen_base) = if a {
                (
                    e.dispcnt.char_base as u32 * 0x1_0000,
                    e.dispcnt.screen_base as u32 * 0x1_0000,
                )
            } else {
                (0, 0)
            };
            let char_base = char_base + u32::from((bgcnt >> 2) & 0xF) * 0x4000;
            let screen_base = screen_base + u32::from((bgcnt >> 8) & 0x1F) * 0x800;
            let size = 128 << size_bits;
            let tiles_per_row = (size / 8) as u32;
            let ext_palette = e.dispcnt.bg_extended_palette;
            let ext_region = if a {
                Region::BgExtPalA
            } else {
                Region::BgExtPalB
            };

            self.affine_walk(a, index, size, size, wrap, |px, x, y| {
                let entry = self
                    .vram
                    .read_u16(region, screen_base + ((y / 8) * tiles_per_row + x / 8) * 2);
                let tile = u32::from(entry & 0x3FF);
                let tx = if entry & (1 << 10) != 0 {
                    7 - (x & 7)
                } else {
                    x & 7
                };
                let ty = if entry & (1 << 11) != 0 {
                    7 - (y & 7)
                } else {
                    y & 7
                };
                let ci = u32::from(
                    self.vram
                        .read_u8(region, char_base + tile * 64 + ty * 8 + tx),
                );
                if ci == 0 {
                    return;
                }
                let color = if ext_palette {
                    let bank = u32::from(entry >> 12);
                    self.vram
                        .read_u16(ext_region, index as u32 * 0x2000 + bank * 512 + ci * 2)
                } else {
                    self.bg_palette_color(a, ci)
                };
                out[px] = OPAQUE | u32::from(color & 0x7FFF);
            });
            return;
        }

        // Bitmap BG: data at screen base * 16 KiB.
        let base = u32::from((bgcnt >> 8) & 0x1F) * 0x4000;
        let (width, height) = match size_bits {
            0 => (128, 128),
            1 => (256, 256),
            2 => (512, 256),
            _ => (512, 512),
        };
        let direct = bgcnt & (1 << 2) != 0;
        self.affine_walk(a, index, width, height, wrap, |px, x, y| {
            let offset = y * width as u32 + x;
            if direct {
                let c = self.vram.read_u16(region, base + offset * 2);
                if c & 0x8000 != 0 {
                    out[px] = OPAQUE | u32::from(c & 0x7FFF);
                }
            } else {
                let ci = u32::from(self.vram.read_u8(region, base + offset));
                if ci != 0 {
                    out[px] = OPAQUE | u32::from(self.bg_palette_color(a, ci) & 0x7FFF);
                }
            }
        });
    }

    /// BG mode 6 large 256-color bitmap (engine A BG2, 512x1024 / 1024x512).
    fn render_bg_large(&self, out: &mut [u32; W]) {
        let bgcnt = self.engine_upper.bgcnt[2];
        let wrap = bgcnt & (1 << 13) != 0;
        let (width, height) = if (bgcnt >> 14) & 1 != 0 {
            (1024, 512)
        } else {
            (512, 1024)
        };
        self.affine_walk(true, 2, width, height, wrap, |px, x, y| {
            let ci = u32::from(self.vram.read_u8(Region::BgA, y * width as u32 + x));
            if ci != 0 {
                out[px] = OPAQUE | u32::from(self.bg_palette_color(true, ci) & 0x7FFF);
            }
        });
    }

    /// Renders every OBJ crossing `line` into `layers.obj` (and the OBJ
    /// window mask). Lower priority values win, ties go to the lower OAM
    /// index.
    fn render_objs(&self, a: bool, line: usize, layers: &mut LineLayers) {
        let oam_base = if a { 0 } else { 0x400 };
        let region = if a { Region::ObjA } else { Region::ObjB };
        let ext_region = if a {
            Region::ObjExtPalA
        } else {
            Region::ObjExtPalB
        };
        let dispcnt = self.engine(a).dispcnt;
        let oam = |off: usize| {
            u16::from_le_bytes([self.oam[oam_base + off], self.oam[oam_base + off + 1]])
        };

        for i in (0..128usize).rev() {
            let attr0 = oam(i * 8);
            let attr1 = oam(i * 8 + 2);
            let attr2 = oam(i * 8 + 4);

            let affine = attr0 & (1 << 8) != 0;
            if !affine && attr0 & (1 << 9) != 0 {
                continue; // OBJ disabled
            }
            let obj_mode = (attr0 >> 10) & 3;
            let (width, height) = OBJ_SIZES[usize::from(attr0 >> 14)][usize::from(attr1 >> 14)];
            let double = affine && attr0 & (1 << 9) != 0;
            let (box_w, box_h) = if double {
                (width * 2, height * 2)
            } else {
                (width, height)
            };

            let y = u32::from(attr0 & 0xFF);
            let row = (line as u32).wrapping_sub(y) & 0xFF;
            if row >= box_h {
                continue;
            }
            let mut x0 = i32::from(attr1 & 0x1FF);
            if x0 >= 256 {
                x0 -= 512;
            }
            let priority = u32::from((attr2 >> 10) & 3);
            let tile = u32::from(attr2 & 0x3FF);
            let pal_bank = u32::from(attr2 >> 12);
            let color_256 = attr0 & (1 << 13) != 0;

            // Affine matrix (identity for normal OBJs, flips folded in).
            let (pa, pb, pc, pd) = if affine {
                let group = usize::from((attr1 >> 9) & 0x1F) * 32;
                (
                    i32::from(oam(group + 6) as i16),
                    i32::from(oam(group + 14) as i16),
                    i32::from(oam(group + 22) as i16),
                    i32::from(oam(group + 30) as i16),
                )
            } else {
                let h = if attr1 & (1 << 12) != 0 { -256 } else { 256 };
                let v = if attr1 & (1 << 13) != 0 { -256 } else { 256 };
                (h, 0, 0, v)
            };

            let half_w = box_w as i32 / 2;
            let half_h = box_h as i32 / 2;
            let ly = row as i32 - half_h;
            for bx in 0..box_w as i32 {
                let sx = x0 + bx;
                if !(0..W as i32).contains(&sx) {
                    continue;
                }
                let lx = bx - half_w;
                let (tx, ty) = if affine {
                    (
                        ((pa * lx + pb * ly) >> 8) + width as i32 / 2,
                        ((pc * lx + pd * ly) >> 8) + height as i32 / 2,
                    )
                } else {
                    let tx = if pa < 0 { width as i32 - 1 - bx } else { bx };
                    let ty = if pd < 0 {
                        height as i32 - 1 - row as i32
                    } else {
                        row as i32
                    };
                    (tx, ty)
                };
                if tx < 0 || ty < 0 || tx >= width as i32 || ty >= height as i32 {
                    continue;
                }
                let (tx, ty) = (tx as u32, ty as u32);

                let color = if obj_mode == 3 {
                    // Bitmap OBJ, 16-bit direct color.
                    if pal_bank == 0 {
                        continue;
                    }
                    let addr = if dispcnt.bitmap_obj_1d {
                        tile * (128 << dispcnt.bitmap_obj_1d_bound as u32) + (ty * width + tx) * 2
                    } else if dispcnt.bitmap_obj_square {
                        (tile & 0x1F) * 0x10 + (tile & 0x3E0) * 0x80 + ty * 512 + tx * 2
                    } else {
                        (tile & 0xF) * 0x10 + (tile & 0x3F0) * 0x80 + ty * 256 + tx * 2
                    };
                    let c = self.vram.read_u16(region, addr);
                    if c & 0x8000 == 0 {
                        continue;
                    }
                    OBJ_BITMAP | (pal_bank << 20) | u32::from(c & 0x7FFF)
                } else {
                    let tile_row = (ty / 8)
                        * if dispcnt.tile_obj_1d {
                            width / 8 * if color_256 { 2 } else { 1 }
                        } else {
                            32
                        };
                    let base = if dispcnt.tile_obj_1d {
                        tile << (5 + dispcnt.tile_obj_1d_bound as u32)
                    } else {
                        tile * 32
                    };
                    let ci = if color_256 {
                        let addr = base + (tile_row + (tx / 8) * 2) * 32 + (ty & 7) * 8 + (tx & 7);
                        u32::from(self.vram.read_u8(region, addr))
                    } else {
                        let addr = base + (tile_row + tx / 8) * 32 + (ty & 7) * 4 + (tx & 7) / 2;
                        let byte = self.vram.read_u8(region, addr);
                        u32::from(if tx & 1 != 0 { byte >> 4 } else { byte & 0xF })
                    };
                    if ci == 0 {
                        continue;
                    }
                    let c = if !color_256 {
                        self.obj_palette_color(a, pal_bank * 16 + ci)
                    } else if dispcnt.obj_extended_palette {
                        self.vram.read_u16(ext_region, pal_bank * 512 + ci * 2)
                    } else {
                        self.obj_palette_color(a, ci)
                    };
                    let semi = if obj_mode == 1 { OBJ_SEMI } else { 0 };
                    semi | u32::from(c & 0x7FFF)
                };

                let sx = sx as usize;
                if obj_mode == 2 {
                    layers.obj_window[sx] = true;
                    continue;
                }
                if !dispcnt.display_obj {
                    continue;
                }
                let existing = layers.obj[sx];
                if existing & OPAQUE == 0 || priority <= (existing >> 16) & 3 {
                    layers.obj[sx] = OPAQUE | (priority << 16) | color;
                }
            }
        }
    }

    /// Window enable mask (bits 0-3 BG0-3, bit 4 OBJ, bit 5 effects) for
    /// every pixel of the line.
    fn window_masks(&self, a: bool, line: usize, layers: &LineLayers) -> [u8; W] {
        let e = self.engine(a);
        let d = e.dispcnt;
        if !(d.display_win0 || d.display_win1 || d.obj_win_display) {
            return [0x3F; W];
        }
        let pack = |bg: [bool; 4], obj: bool, fx: bool| -> u8 {
            bg.iter()
                .enumerate()
                .fold(0u8, |m, (i, &on)| m | (u8::from(on) << i))
                | (u8::from(obj) << 4)
                | (u8::from(fx) << 5)
        };
        let win0 = pack(
            e.winin.win0_bg_enabled,
            e.winin.win0_obj_enabled,
            e.winin.win0_color_special,
        );
        let win1 = pack(
            e.winin.win1_bg_enabled,
            e.winin.win1_obj_enabled,
            e.winin.win1_color_special,
        );
        let outside = pack(
            e.winout.outside_bg_enabled,
            e.winout.outside_obj_enabled,
            e.winout.outside_color_special,
        );
        let objwin = pack(
            e.winout.objwin_bg_enabled,
            e.winout.objwin_obj_enabled,
            e.winout.objwin_color_special,
        );

        let line = line as u32;
        let v_in = |v: u16| in_span(line, u32::from(v >> 8), u32::from(v & 0xFF));
        let w0_v = d.display_win0 && v_in(e.win0v);
        let w1_v = d.display_win1 && v_in(e.win1v);

        let mut masks = [outside; W];
        for (x, m) in masks.iter_mut().enumerate() {
            let x32 = x as u32;
            if w0_v && in_span(x32, u32::from(e.win0h >> 8), u32::from(e.win0h & 0xFF)) {
                *m = win0;
            } else if w1_v && in_span(x32, u32::from(e.win1h >> 8), u32::from(e.win1h & 0xFF)) {
                *m = win1;
            } else if d.obj_win_display && layers.obj_window[x] {
                *m = objwin;
            }
        }
        masks
    }

    /// Picks the top two visible layers per pixel and applies BLDCNT.
    fn composite(&mut self, a: bool, line: usize, layers: &LineLayers) {
        let masks = self.window_masks(a, line, layers);
        let backdrop = self.bg_palette_color(a, 0) & 0x7FFF;
        let e = self.engine(a);
        let bgcnt = e.bgcnt;
        let bld = e.bldcnt;
        let eva = u32::from(e.bldalpha & 0x1F).min(16);
        let evb = u32::from((e.bldalpha >> 8) & 0x1F).min(16);
        let evy = u32::from(e.bldy & 0x1F).min(16);
        let first_target = |layer: usize| match layer {
            0..=3 => bld.bg_first_target_pix[layer],
            4 => bld.obj_first_target_pix,
            _ => bld.bd_first_target_pix,
        };
        let second_target = |layer: usize| match layer {
            0..=3 => bld.bg_second_target_pix[layer],
            4 => bld.obj_second_target_pix,
            _ => bld.bd_second_target_pix,
        };

        let mut row = [0u32; W];
        for x in 0..W {
            let mask = masks[x];
            // (layer id, pixel); layer 5 = backdrop.
            let mut found = [(5usize, u32::from(backdrop)); 2];
            let mut count = 0;
            'search: for prio in 0..4u32 {
                let obj = layers.obj[x];
                if obj & OPAQUE != 0 && (obj >> 16) & 3 == prio && mask & 0x10 != 0 {
                    found[count] = (4, obj);
                    count += 1;
                    if count == 2 {
                        break 'search;
                    }
                }
                for (bg, bg_line) in layers.bg.iter().enumerate() {
                    let px = bg_line[x];
                    if px & OPAQUE != 0 && u32::from(bgcnt[bg] & 3) == prio && mask & (1 << bg) != 0
                    {
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
            let top_color = (top & 0x7FFF) as u16;
            let second_color = (second & 0x7FFF) as u16;
            let effects = mask & 0x20 != 0;

            let color = if !effects {
                top_color
            } else if top_layer == 4
                && top & (OBJ_SEMI | OBJ_BITMAP) != 0
                && second_target(second_layer)
            {
                // Semi-transparent / bitmap OBJs blend regardless of BLDCNT mode.
                if top & OBJ_BITMAP != 0 {
                    let alpha = ((top >> 20) & 0xF) + 1;
                    blend(top_color, second_color, alpha, 16 - alpha)
                } else {
                    blend(top_color, second_color, eva, evb)
                }
            } else if top_layer == 0 && top & BG_3D != 0 && second_target(second_layer) {
                // 3D pixels carry their own alpha; they are opaque here.
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
        }

        let start = line * W;
        self.engine_mut(a).framebuffer[start..start + W].copy_from_slice(&row);
    }
}
