//! BG and OBJ mosaic (MOSAIC register, BGxCNT bit 6, OBJ attribute 0
//! bit 12).
//!
//! ```text
//!  MOSAIC  bits 0-3 BG width-1   4-7 BG height-1   8-11 OBJ width-1   12-15 OBJ height-1
//!
//!  vertical:   a counter runs per line; while it is inside a block the
//!              lines repeat the block's first line (`bg_line` / `obj_line`;
//!              affine BGs hold their reference point instead)
//!  horizontal: the grid starts at screen x = 0; every pixel takes the
//!              value of the first pixel of its block
//!
//!   x: 0 1 2 3 4 5 6 7       width 3 (MOSAIC = 2)
//!      A B C D E F G H  -->  A A A D D D G G
//! ```
//!
//! Follows melonDS (`GPU2D::UpdateRegistersPreDraw/PostDraw`,
//! `SoftRenderer2D::ApplySpriteMosaicX`). GBATEK:
//! [LCD I/O Mosaic Function](https://problemkaputt.de/gbatek.htm#lcdiomosaicfunction)

use super::Gpu2DEngine;
use crate::engine_2d::compose::{OBJ_MOSAIC, OPAQUE, W};

/// Per-line mosaic counters of one engine.
#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone, Copy)]
pub struct MosaicState {
    bg_y: u8,
    /// BG height latched at the start of each block (a mid-block MOSAIC
    /// write applies from the next block).
    bg_y_max: u8,
    bg_latch: bool,
    /// First line of the current BG block.
    pub(super) bg_line: u32,
    obj_y: u8,
    /// First line of the current OBJ block.
    pub(super) obj_line: u32,
    /// BG2/BG3 reference points (x, y) drawn this line; a mosaic BG keeps
    /// them for the whole block.
    pub(super) affine_ref: [(i32, i32); 2],
}

impl Gpu2DEngine {
    /// MOSAIC BG block size minus one: (width, height).
    pub(super) const fn bg_mosaic_size(&self) -> (u32, u32) {
        ((self.mosaic & 0xF) as u32, ((self.mosaic >> 4) & 0xF) as u32)
    }

    /// MOSAIC OBJ block size minus one: (width, height).
    pub(super) const fn obj_mosaic_size(&self) -> (u32, u32) {
        (((self.mosaic >> 8) & 0xF) as u32, ((self.mosaic >> 12) & 0xF) as u32)
    }

    /// Whether BGxCNT bit 6 (mosaic) is set.
    pub(super) const fn bg_mosaic(&self, index: usize) -> bool {
        self.bgcnt[index] & (1 << 6) != 0
    }

    /// Before drawing `line`: latch the block's first lines and the
    /// affine reference points.
    pub(super) fn mosaic_line_start(&mut self, line: usize) {
        let (_, bg_h) = self.bg_mosaic_size();
        let (_, obj_h) = self.obj_mosaic_size();
        let m = &mut self.mosaic_state;
        if line == 0 {
            m.bg_y = 0;
            m.bg_y_max = bg_h as u8;
            m.bg_latch = true;
        }
        if m.bg_latch {
            m.bg_line = line as u32;
        }

        if line == 0 || u32::from(m.obj_y) == obj_h {
            m.obj_y = 0;
            m.obj_line = line as u32;
        } else {
            m.obj_y = (m.obj_y + 1) & 0xF;
        }

        let refs =
            [(self.bg2x_internal, self.bg2y_internal), (self.bg3x_internal, self.bg3y_internal)];
        let latch = self.mosaic_state.bg_latch;
        for (i, r) in refs.into_iter().enumerate() {
            if !self.bg_mosaic(2 + i) || latch {
                self.mosaic_state.affine_ref[i] = r;
            }
        }
    }

    /// After drawing a line: advance the BG block counter.
    pub(super) fn mosaic_line_end(&mut self) {
        let (_, bg_h) = self.bg_mosaic_size();
        let m = &mut self.mosaic_state;
        if m.bg_y == m.bg_y_max {
            m.bg_y_max = bg_h as u8;
            m.bg_y = 0;
            m.bg_latch = true;
        } else {
            m.bg_y = (m.bg_y + 1) & 0xF;
            m.bg_latch = false;
        }
    }
}

/// Horizontal BG mosaic: each pixel repeats the first pixel of its block.
pub(super) fn bg_mosaic_x(line: &mut [u32; W], width: u32) {
    if width == 0 {
        return;
    }
    let block = width as usize + 1;
    for x in 0..W {
        line[x] = line[x - x % block];
    }
}

/// Horizontal OBJ mosaic. Going left to right, a pixel latches its own
/// value at the start of a block, when it or the latched one is not a
/// mosaic OBJ pixel, or when it has a higher priority (lower value);
/// otherwise it repeats the latched one.
pub(super) fn obj_mosaic_x(obj: &mut [u32; W], width: u32) {
    if width == 0 {
        return;
    }
    let prio = |p: u32| (p >> 16) & 3;
    let mut latched = 0;
    let mut mx = 0;
    for px in obj.iter_mut() {
        let cur = *px;
        let mosaic = |p: u32| p & (OPAQUE | OBJ_MOSAIC) == OPAQUE | OBJ_MOSAIC;
        if mx == 0 || !mosaic(cur) || !mosaic(latched) || prio(cur) < prio(latched) {
            latched = cur;
        }
        *px = latched;
        mx = if mx == width { 0 } else { mx + 1 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bg_mosaic_repeats_block_start() {
        let mut line = [0; W];
        for (i, p) in line.iter_mut().enumerate() {
            *p = i as u32;
        }
        bg_mosaic_x(&mut line, 2);
        assert_eq!(&line[..8], &[0, 0, 0, 3, 3, 3, 6, 6]);
    }

    #[test]
    fn obj_mosaic_only_spreads_mosaic_pixels() {
        let mut obj = [0; W];
        let m = OPAQUE | OBJ_MOSAIC;
        obj[0] = m | 1;
        obj[1] = m | 2;
        obj[2] = OPAQUE | 3; // plain OBJ pixel: kept
        obj[3] = m | 4;
        obj_mosaic_x(&mut obj, 3);
        assert_eq!(obj[1], m | 1);
        assert_eq!(obj[2], OPAQUE | 3);
        assert_eq!(obj[3], m | 4, "latched again after a plain pixel");
    }
}
