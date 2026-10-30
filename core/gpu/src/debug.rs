//! Debugger views of video memory: palettes, BG maps, tile sheets and raw
//! VRAM banks, drawn straight from VRAM (not from the screen output).
//!
//! ```text
//!  render_palettes  16x16 swatches of a standard or extended palette
//!  render_map       a whole BG map (text: 16-bit entries, affine: 8-bit)
//!  render_tiles     a 128 KiB window of BG/OBJ VRAM as 4/8bpp tiles or
//!                   a direct-color bitmap
//!  render_bank      one VRAM bank (A-I) as BGR555 pixels
//! ```
//!
//! Every image is BGR555 per pixel with bit 15 set where it is opaque, plus
//! its width and height.
use crate::{
    Gpu,
    vram::{BANK_SIZES, Region},
};

/// One of the two 2D engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// Engine A: 2D and 3D, main screen.
    A,
    /// Engine B: 2D only, sub screen.
    B,
}

impl Engine {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }

    const fn is_a(self) -> bool {
        matches!(self, Self::A)
    }
}

/// Backgrounds or sprites (OBJ).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsType {
    BG,
    OBJ,
}

impl GraphicsType {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::BG => "BG",
            Self::OBJ => "OBJ",
        }
    }
}

/// A debug image: BGR555 pixels (bit 15 = opaque), width, height.
pub type DebugImage = (Vec<u16>, usize, usize);

const OPAQUE: u16 = 0x8000;

/// 16x16 palette entries as 8x8-pixel swatches.
fn swatches(color: impl Fn(usize) -> u16) -> DebugImage {
    const N: usize = 16;
    let size = N * 8;
    let mut pixels = vec![0; size * size];
    for (i, px) in pixels.iter_mut().enumerate() {
        let (x, y) = (i % size, i / size);
        *px = color((y / 8) * N + x / 8) | OPAQUE;
    }
    (pixels, size, size)
}

impl Gpu {
    /// BG / OBJ palette color `index` of `engine` (standard palettes).
    fn std_palette(&self, engine: Engine, kind: GraphicsType, index: usize) -> u16 {
        match kind {
            GraphicsType::BG => self.bg_palette_color(engine.is_a(), index as u32),
            GraphicsType::OBJ => self.obj_palette_color(engine.is_a(), index as u32),
        }
    }

    /// Extended palette color `index` (palette bank * 256 + color): BG slot
    /// `slot` (0-3), or the OBJ extended palette.
    fn ext_palette(&self, engine: Engine, kind: GraphicsType, slot: usize, index: usize) -> u16 {
        // DS Video Extended Palettes: https://problemkaputt.de/gbatek.htm#dsvideoextendedpalettes
        let (region, offset) = match (kind, engine) {
            (GraphicsType::BG, Engine::A) => (Region::BgExtPalA, slot * 0x2000),
            (GraphicsType::BG, Engine::B) => (Region::BgExtPalB, slot * 0x2000),
            (GraphicsType::OBJ, Engine::A) => (Region::ObjExtPalA, 0),
            (GraphicsType::OBJ, Engine::B) => (Region::ObjExtPalB, 0),
        };
        self.vram.read_u16(region, (offset + index * 2) as u32)
    }

    /// A palette: the standard 256 colors, or bank `palette` of an extended
    /// one (BG: slot `slot`).
    pub fn render_palettes(
        &self,
        extended: bool,
        slot: usize,
        palette: usize,
        engine: Engine,
        kind: GraphicsType,
    ) -> DebugImage {
        if extended {
            swatches(|i| self.ext_palette(engine, kind, slot, palette * 256 + i))
        } else {
            swatches(|i| self.std_palette(engine, kind, i))
        }
    }

    /// Whether BG `bg` of `engine` is drawn as an affine (8-bit map) BG in
    /// the current BG mode.
    fn bg_is_affine(&self, engine: Engine, bg: usize) -> bool {
        // DS Video BG Modes / Control: https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol
        match self.engine(engine.is_a()).dispcnt.bg_mode {
            1 => bg == 3,
            2 => bg >= 2,
            4 => bg == 2,
            _ => false,
        }
    }

    /// The whole map of BG `bg` (0-3), at every size BGxCNT selects.
    pub fn render_map(&self, engine: Engine, bg: usize) -> DebugImage {
        let a = engine.is_a();
        let e = self.engine(a);
        let bg = bg.min(3);
        let bgcnt = e.bgcnt[bg];
        let affine = self.bg_is_affine(engine, bg);
        let size = usize::from((bgcnt >> 14) & 3);
        let (width, height) = if affine {
            [(128, 128), (256, 256), (512, 512), (1024, 1024)][size]
        } else {
            [(256, 256), (512, 256), (256, 512), (512, 512)][size]
        };
        let region = if a { Region::BgA } else { Region::BgB };
        let (char_base, screen_base) = if a {
            (e.dispcnt.char_base as u32 * 0x1_0000, e.dispcnt.screen_base as u32 * 0x1_0000)
        } else {
            (0, 0)
        };
        // LCD I/O BG Control: https://problemkaputt.de/gbatek.htm#lcdiobgcontrol
        let char_base = char_base + u32::from((bgcnt >> 2) & 0xF) * 0x4000;
        let screen_base = screen_base + u32::from((bgcnt >> 8) & 0x1F) * 0x800;
        let color_256 = affine || bgcnt & (1 << 7) != 0;
        let ext = !affine && color_256 && e.dispcnt.bg_extended_palette;
        let ext_slot = if bg < 2 && bgcnt & (1 << 13) != 0 { bg + 2 } else { bg };

        let mut pixels = vec![0_u16; width * height];
        for (i, px) in pixels.iter_mut().enumerate() {
            let (x, y) = ((i % width) as u32, (i / width) as u32);
            let (tile, tx, ty, pal_bank) = if affine {
                let entry =
                    self.vram.read_u8(region, screen_base + (y / 8) * (width as u32 / 8) + x / 8);
                (u32::from(entry), x & 7, y & 7, 0)
            } else {
                // 32x32-entry blocks, 2K apart (see `engine_2d::bg`).
                let block = match size {
                    1 => (x / 256) * 0x800,
                    2 => (y / 256) * 0x800,
                    3 => (y / 256) * 0x1000 + (x / 256) * 0x800,
                    _ => 0,
                };
                let entry = self.vram.read_u16(
                    region,
                    screen_base + block + ((y / 8) & 31) * 64 + ((x / 8) & 31) * 2,
                );
                let tx = if entry & (1 << 10) != 0 { 7 - (x & 7) } else { x & 7 };
                let ty = if entry & (1 << 11) != 0 { 7 - (y & 7) } else { y & 7 };
                (u32::from(entry & 0x3FF), tx, ty, u32::from(entry >> 12))
            };
            let color = if color_256 {
                let ci = u32::from(self.vram.read_u8(region, char_base + tile * 64 + ty * 8 + tx));
                if ci == 0 {
                    continue;
                }
                if ext {
                    self.ext_palette(
                        engine,
                        GraphicsType::BG,
                        ext_slot,
                        (pal_bank * 256 + ci) as usize,
                    )
                } else {
                    self.bg_palette_color(a, ci)
                }
            } else {
                let byte = self.vram.read_u8(region, char_base + tile * 32 + ty * 4 + tx / 2);
                let ci = u32::from(if tx & 1 != 0 { byte >> 4 } else { byte & 0xF });
                if ci == 0 {
                    continue;
                }
                self.bg_palette_color(a, pal_bank * 16 + ci)
            };
            *px = color | OPAQUE;
        }
        (pixels, width, height)
    }

    /// 128 KiB of BG/OBJ VRAM from `offset` * 128 KiB on: a 256x256
    /// direct-color bitmap (`bitmap`), or 4bpp / 8bpp (`bpp8` or `extended`)
    /// tiles colored with palette bank `palette` (extended: of BG slot
    /// `slot` / the OBJ extended palette).
    #[expect(clippy::too_many_arguments, reason = "one parameter per debugger control")]
    pub fn render_tiles(
        &self,
        engine: Engine,
        kind: GraphicsType,
        extended: bool,
        bitmap: bool,
        bpp8: bool,
        slot: usize,
        palette: usize,
        offset: usize,
    ) -> DebugImage {
        let region = match (kind, engine) {
            (GraphicsType::BG, Engine::A) => Region::BgA,
            (GraphicsType::BG, Engine::B) => Region::BgB,
            (GraphicsType::OBJ, Engine::A) => Region::ObjA,
            (GraphicsType::OBJ, Engine::B) => Region::ObjB,
        };
        let start = (offset * 128 * 0x400) as u32;
        if bitmap {
            let (width, height) = (256, 256);
            let pixels = (0..width * height)
                .map(|i| self.vram.read_u16(region, start + 2 * i as u32))
                .collect();
            return (pixels, width, height);
        }
        let bpp8 = bpp8 || extended;
        let (tiles_w, tiles_h) = if bpp8 { (64, 32) } else { (64, 64) };
        let (width, height) = (tiles_w * 8, tiles_h * 8);
        let tile_bytes = if bpp8 { 64 } else { 32 };
        let mut pixels = vec![0_u16; width * height];
        for (i, px) in pixels.iter_mut().enumerate() {
            let (x, y) = (i % width, i / width);
            let tile = (y / 8) * tiles_w + x / 8;
            let (tx, ty) = (x % 8, y % 8);
            let addr = start + (tile * tile_bytes) as u32;
            let ci = if bpp8 {
                usize::from(self.vram.read_u8(region, addr + (ty * 8 + tx) as u32))
            } else {
                let byte = self.vram.read_u8(region, addr + (ty * 4 + tx / 2) as u32);
                usize::from(if tx & 1 != 0 { byte >> 4 } else { byte & 0xF })
            };
            if ci == 0 {
                continue;
            }
            let color = if extended {
                self.ext_palette(engine, kind, slot, palette * 256 + ci)
            } else if bpp8 {
                self.std_palette(engine, kind, ci)
            } else {
                self.std_palette(engine, kind, palette * 16 + ci)
            };
            *px = color | OPAQUE;
        }
        (pixels, width, height)
    }

    /// VRAM bank `bank` (0 = A ... 8 = I) as BGR555, 256 pixels wide;
    /// `ignore_alpha` shows every pixel as opaque.
    pub fn render_bank(&self, bank: usize, ignore_alpha: bool) -> DebugImage {
        let bank = bank.min(BANK_SIZES.len() - 1);
        let alpha = if ignore_alpha { OPAQUE } else { 0 };
        let pixels: Vec<u16> = self
            .vram
            .bank(bank)
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b) | alpha)
            .collect();
        let width = 256;
        let height = pixels.len() / width;
        (pixels, width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_sizes() {
        let gpu = Gpu::new();
        let (p, w, h) = gpu.render_palettes(false, 0, 0, Engine::A, GraphicsType::BG);
        assert_eq!((p.len(), w, h), (128 * 128, 128, 128));
        let (p, w, h) = gpu.render_map(Engine::B, 0);
        assert_eq!((p.len(), w, h), (256 * 256, 256, 256));
        let (_, w, h) = gpu.render_tiles(Engine::A, GraphicsType::OBJ, false, false, true, 0, 0, 0);
        assert_eq!((w, h), (512, 256));
        let (p, w, h) = gpu.render_bank(4, true);
        assert_eq!((w, h), (256, 128), "bank E is 64 KiB");
        assert!(p.iter().all(|px| px & OPAQUE != 0));
    }

    #[test]
    fn bank_pixels_come_from_vram() {
        let mut gpu = Gpu::new();
        gpu.vram.bank_mut(0)[0..2].copy_from_slice(&0x1234_u16.to_le_bytes());
        assert_eq!(gpu.render_bank(0, false).0[0], 0x1234);
    }
}
