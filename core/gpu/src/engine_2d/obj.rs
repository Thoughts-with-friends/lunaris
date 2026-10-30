//! Sprites (OBJ): normal, affine and bitmap OBJs, plus the OBJ window.
//! See GBATEK "DS Video OBJs".
//!
//! ## Sprites (OBJ)
//!
//! OAM holds 128 sprites per engine, 8 bytes each: three attributes plus one
//! 16-bit slot of a rotation/scaling parameter group.
//!
//! ```text
//!   attr0  bits 0-7 Y          8 rot/scale on     9 double size (rot/scale on)
//!                                                   or hide (rot/scale off)
//!          10-11 mode          12 mosaic          13 256 colors   14-15 shape
//!   attr1  bits 0-8 X          9-13 parameter group (rot/scale on)
//!                              12 H flip, 13 V flip (rot/scale off)
//!                              14-15 size
//!   attr2  bits 0-9 tile       10-11 priority     12-15 palette bank
//!                                                   (bitmap OBJ: alpha)
//!
//!   shape \ size   0       1       2       3
//!   square         8x8     16x16   32x32   64x64
//!   wide           16x8    32x8    32x16   64x32
//!   tall           8x16    8x32    16x32   32x64
//!
//!   screen 256x192
//!   +------------------------------+   X is 9 bits (256-511 = -256..-1),
//!   |  (X,Y)                       |   Y is 8 bits and wraps at 256,
//!   |    +-------+                 |   so a sprite can slide in from any edge
//!   |    | 16x16 |                 |
//!   |    +-------+                 |
//!   +------------------------------+
//! ```
//!
//! Rotation/scaling: each pixel (lx, ly) of the sprite's box, measured from
//! the box center, is mapped back into the sprite image:
//!
//! ```text
//!   texX = (PA*lx + PB*ly) / 256 + width/2      PA..PD: group n = attr slot 3
//!   texY = (PC*lx + PD*ly) / 256 + height/2     of OAM entries 4n..4n+3
//!
//!   "double size" doubles the box so a rotated sprite is not cut off:
//!
//!     normal box            double-size box
//!     +-------+             +---------------+
//!     |   /\  |             |               |
//!     |  /  \ | <- corners  |      /\       |
//!     |  \  / |    cut off  |     /  \      |
//!     |   \/  |             |     \  /      |
//!     +-------+             |      \/       |
//!                           +---------------+
//! ```
//!
//! ```text
//!   OBJ mode (attr0 bits 10-11): 0 normal, 1 semi-transparent (alpha blended
//!   with EVA/EVB), 2 OBJ window (shape only, see window.rs), 3 bitmap
//!
//!   tile layout in OBJ VRAM (DISPCNT bit 4):
//!     2D mapping: tiles form a sheet 32 tiles wide; a 16x16 4bpp sprite at
//!                 tile t uses  t   t+1
//!                              t+32 t+33
//!     1D mapping: a sprite's tiles are consecutive: t, t+1, t+2, t+3
//!
//!   overlapping sprites: lower priority value wins, then lower OAM index
//! ```
//!
//! GBATEK: [LCD OBJ - OAM Attributes](https://problemkaputt.de/gbatek.htm#lcdobjoamattributes),
//! [LCD OBJ - OAM Rotation/Scaling Parameters](https://problemkaputt.de/gbatek.htm#lcdobjoamrotationscalingparameters),
//! [DS Video OBJs](https://problemkaputt.de/gbatek.htm#dsvideoobjs)
use super::compose::{LineLayers, OBJ_BITMAP, OBJ_SEMI, OPAQUE, W};
use crate::{Gpu, vram::Region};

/// OBJ sizes in pixels: `[shape][size] = (width, height)`.
const OBJ_SIZES: [[(u32, u32); 4]; 4] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)],
    [(16, 8), (32, 8), (32, 16), (64, 32)],
    [(8, 16), (8, 32), (16, 32), (32, 64)],
    [(8, 8), (8, 8), (8, 8), (8, 8)], // prohibited shape
];

impl Gpu {
    /// Renders every OBJ crossing `line` into `layers.obj` (and the OBJ
    /// window mask). Lower priority values win, ties go to the lower OAM
    /// index.
    pub(super) fn render_objs(&self, a: bool, line: usize, layers: &mut LineLayers) {
        // LCD OBJ - OAM Attributes: https://problemkaputt.de/gbatek.htm#lcdobjoamattributes
        let oam_base = if a { 0 } else { 0x400 }; // engine B OAM starts at offset 400h
        let region = if a { Region::ObjA } else { Region::ObjB };
        let ext_region = if a { Region::ObjExtPalA } else { Region::ObjExtPalB };
        let dispcnt = self.engine(a).dispcnt;
        let oam = |off: usize| {
            u16::from_le_bytes([self.oam[oam_base + off], self.oam[oam_base + off + 1]])
        };

        for i in (0..128usize).rev() {
            let attr0 = oam(i * 8);
            let attr1 = oam(i * 8 + 2);
            let attr2 = oam(i * 8 + 4);

            // LCD OBJ - OAM Attributes: https://problemkaputt.de/gbatek.htm#lcdobjoamattributes
            let affine = attr0 & (1 << 8) != 0; // attribute 0 bit 8: rotation/scaling
            if !affine && attr0 & (1 << 9) != 0 {
                // attribute 0 bit 9 without rot/scale: OBJ disabled
                continue; // OBJ disabled
            }
            let obj_mode = (attr0 >> 10) & 3;
            let (width, height) = OBJ_SIZES[usize::from(attr0 >> 14)][usize::from(attr1 >> 14)];
            let double = affine && attr0 & (1 << 9) != 0; // attribute 0 bit 9 with rot/scale: double size
            let (box_w, box_h) = if double { (width * 2, height * 2) } else { (width, height) };

            let y = u32::from(attr0 & 0xFF); // attribute 0: Y coordinate (bits 0-7)
            let row = (line as u32).wrapping_sub(y) & 0xFF; // row within the sprite, wrapping at 256
            if row >= box_h {
                continue;
            }
            let mut x0 = i32::from(attr1 & 0x1FF); // attribute 1: X coordinate (bits 0-8)
            if x0 >= 256 {
                x0 -= 512;
            }
            let priority = u32::from((attr2 >> 10) & 3);
            let tile = u32::from(attr2 & 0x3FF); // attribute 2: tile number (bits 0-9)
            let pal_bank = u32::from(attr2 >> 12);
            let color_256 = attr0 & (1 << 13) != 0; // attribute 0 bit 13: 256 colors

            // Affine matrix (identity for normal OBJs, flips folded in).
            let (pa, pb, pc, pd) = if affine {
                let group = usize::from((attr1 >> 9) & 0x1F) * 32; // attribute 1: rotation/scaling parameter group (bits 9-13)
                (
                    i32::from(oam(group + 6) as i16),
                    i32::from(oam(group + 14) as i16),
                    i32::from(oam(group + 22) as i16),
                    i32::from(oam(group + 30) as i16),
                )
            } else {
                let h = if attr1 & (1 << 12) != 0 { -256 } else { 256 }; // attribute 1 bit 12: horizontal flip
                let v = if attr1 & (1 << 13) != 0 { -256 } else { 256 }; // attribute 1 bit 13: vertical flip
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
                    let ty = if pd < 0 { height as i32 - 1 - row as i32 } else { row as i32 };
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
                        // DS Video OBJs: https://problemkaputt.de/gbatek.htm#dsvideoobjs
                        (tile & 0x1F) * 0x10 + (tile & 0x3E0) * 0x80 + ty * 512 + tx * 2 // 2D bitmap OBJ, 256-pixel wide VRAM layout
                    } else {
                        (tile & 0xF) * 0x10 + (tile & 0x3F0) * 0x80 + ty * 256 + tx * 2 // 2D bitmap OBJ, 128-pixel wide VRAM layout
                    };
                    let c = self.vram.read_u16(region, addr);
                    if c & 0x8000 == 0 {
                        // bitmap OBJ bit 15 = alpha
                        continue;
                    }
                    OBJ_BITMAP | (pal_bank << 20) | u32::from(c & 0x7FFF) // BGR555 color
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
                        u32::from(if tx & 1 != 0 { byte >> 4 } else { byte & 0xF }) // 4bpp: low nibble for even pixels, high nibble for odd
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
                    semi | u32::from(c & 0x7FFF) // BGR555 color
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
                    layers.obj[sx] = OPAQUE | (priority << 16) | color; // layer priority in bits 16-17
                }
            }
        }
    }
}
