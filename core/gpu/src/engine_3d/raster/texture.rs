//! Texture sampling: wrap / mirror / clamp and the 7 texture formats
//! (melonDS `TextureLookup`). See GBATEK "DS 3D Texture Formats".
//!
//! ```text
//!   texture coordinates S, T (12.4 fixed point, interpolated per pixel)
//!   -> texel (s, t) in a texture of 8..1024 x 8..1024 texels
//!      (TEXIMAGE_PARAM bits 20-22 width, 23-25 height)
//!
//!   outside 0..size-1 (per axis):
//!     repeat (bit 16/17)          0 1 2 3 | 0 1 2 3 | 0 1 2 3
//!     repeat + flip (bit 18/19)   0 1 2 3 | 3 2 1 0 | 0 1 2 3
//!     clamp (no repeat)           0 0 0 0 | 0 1 2 3 | 3 3 3 3
//!                                 (left)    texture   (right)
//!
//!   formats (TEXIMAGE_PARAM bits 26-28):
//!     1 A3I5        8 bits/texel: color index 0-4, alpha 5-7
//!     2 4-color     2 bits/texel
//!     3 16-color    4 bits/texel
//!     4 256-color   8 bits/texel
//!     5 4x4 compressed: each 4x4 block = 32 bits of 2-bit indices (slot 0/2)
//!                       + 16 bits in slot 1 choosing palette + mode
//!                       (2 colors + mixes, or 3 colors + transparent, ...)
//!     6 A5I3        8 bits/texel: color index 0-2, alpha 3-7
//!     7 direct      16 bits/texel, BGR555 + bit 15 alpha
//!   palette formats read colors from texture palette VRAM at PLTT_BASE;
//!   bit 29 makes color 0 transparent (formats 2-4).
//!
//!   texture VRAM is 512K (4 slots of 128K), mapped from VRAM banks A-D.
//! ```
//!
//! GBATEK: [DS 3D Texture Formats](https://problemkaputt.de/gbatek.htm#ds3dtextureformats),
//! [DS 3D Texture Attributes](https://problemkaputt.de/gbatek.htm#ds3dtextureattributes)
use super::{Frame, PAL_SIZE, TEX_SIZE};

impl Frame<'_> {
    /// Texel color (BGR555) and alpha (0-31), melonDS `TextureLookup`.
    pub(super) fn texture_lookup(&self, texparam: u32, texpal: u32, s: i16, t: i16) -> (u32, u32) {
        let tex = |a: u32| self.tex[a as usize & (TEX_SIZE - 1)];
        let tex16 = |a: u32| {
            let a = a as usize & (TEX_SIZE - 2);
            u32::from(u16::from_le_bytes([self.tex[a], self.tex[a + 1]]))
        };
        let pal = |a: u32| {
            let a = a as usize & (PAL_SIZE - 2);
            u32::from(u16::from_le_bytes([self.pal[a], self.pal[a + 1]]))
        };

        // DS 3D Texture Attributes: https://problemkaputt.de/gbatek.htm#ds3dtextureattributes
        let mut vramaddr = (texparam & 0xFFFF) << 3; // TEXIMAGE_PARAM texture VRAM offset (bits 0-15) in 8-byte units
        let width = 8_i32 << ((texparam >> 20) & 7);
        let height = 8_i32 << ((texparam >> 23) & 7);
        let mut s = i32::from(s >> 4);
        let mut t = i32::from(t >> 4);

        // DS 3D Texture Attributes: https://problemkaputt.de/gbatek.htm#ds3dtextureattributes
        if texparam & (1 << 16) != 0 {
            // TEXIMAGE_PARAM bit 16: repeat in S direction
            if texparam & (1 << 18) != 0 && s & width != 0 {
                // TEXIMAGE_PARAM bit 18: flip in S direction
                s = (width - 1) - (s & (width - 1));
            } else {
                s &= width - 1;
            }
        } else {
            s = s.clamp(0, width - 1);
        }
        if texparam & (1 << 17) != 0 {
            // TEXIMAGE_PARAM bit 17: repeat in T direction
            if texparam & (1 << 19) != 0 && t & height != 0 {
                // TEXIMAGE_PARAM bit 19: flip in T direction
                t = (height - 1) - (t & (height - 1));
            } else {
                t &= height - 1;
            }
        } else {
            t = t.clamp(0, height - 1);
        }
        let (s, t, width) = (s as u32, t as u32, width as u32);

        let alpha0 = if texparam & (1 << 29) != 0 { 0 } else { 31 }; // TEXIMAGE_PARAM bit 29: color 0 is transparent
        match (texparam >> 26) & 7 {
            1 => {
                // A3I5
                let pixel = u32::from(tex(vramaddr + t * width + s));
                // DS 3D Texture Formats: https://problemkaputt.de/gbatek.htm#ds3dtextureformats
                let color = pal((texpal << 4) + ((pixel & 0x1F) << 1)); // A3I5: palette index bits 0-4
                (color, ((pixel >> 3) & 0x1C) + (pixel >> 6)) // A3I5: 3-bit alpha (bits 5-7) expanded to 5 bits
            }
            2 => {
                // 4-color palette
                let pixel =
                    (u32::from(tex(vramaddr + ((t * width + s) >> 2))) >> ((s & 3) << 1)) & 3;
                let color = pal((texpal << 3) + (pixel << 1));
                (color, if pixel == 0 { alpha0 } else { 31 })
            }
            3 => {
                // 16-color palette
                let byte = u32::from(tex(vramaddr + ((t * width + s) >> 1)));
                let pixel = if s & 1 != 0 { byte >> 4 } else { byte & 0xF }; // 4-color/16-color: low nibble for even texels, high nibble for odd
                let color = pal((texpal << 4) + (pixel << 1));
                (color, if pixel == 0 { alpha0 } else { 31 })
            }
            4 => {
                // 256-color palette
                let pixel = u32::from(tex(vramaddr + t * width + s));
                let color = pal((texpal << 4) + (pixel << 1));
                (color, if pixel == 0 { alpha0 } else { 31 })
            }
            5 => {
                // 4x4 compressed
                vramaddr += (t & 0x3FC) * (width >> 2) + (s & 0x3FC);
                vramaddr += t & 3;
                vramaddr &= 0x7FFFF; // texture VRAM is 512K
                let mut slot1 = 0x20000 + ((vramaddr & 0x1FFFC) >> 1); // 4x4 compressed: palette index data in slot 1 (20000h)
                if vramaddr >= 0x40000 {
                    // texels in slot 2 use the upper half of slot 1
                    slot1 += 0x10000; // second 64K of slot 1
                }
                let val = if (0x20000..0x40000).contains(&vramaddr) {
                    // slot 1 holds palette index data, not texels
                    0
                } else {
                    u32::from(tex(vramaddr)) >> (2 * (s & 3))
                };
                let palinfo = tex16(slot1);
                let base = (texpal << 4) + ((palinfo & 0x3FFF) << 2); // 4x4 compressed: palette offset (bits 0-13) * 4
                let mix = |w0: u32, w1: u32| {
                    let (c0, c1) = (pal(base), pal(base + 2));
                    let r = ((c0 & 0x1F) * w0 + (c1 & 0x1F) * w1) >> 3; // blend red (bits 0-4)
                    let g = (((c0 & 0x3E0) * w0 + (c1 & 0x3E0) * w1) >> 3) & 0x3E0; // blend green (bits 5-9)
                    let b = (((c0 & 0x7C00) * w0 + (c1 & 0x7C00) * w1) >> 3) & 0x7C00; // blend blue (bits 10-14)
                    r | g | b
                };
                match (val & 3, palinfo >> 14) {
                    (0, _) => (pal(base), 31),
                    (1, _) => (pal(base + 2), 31),
                    (2, 1) => (mix(4, 4), 31),
                    (2, 3) => (mix(5, 3), 31),
                    (2, _) => (pal(base + 4), 31),
                    (3, 2) => (pal(base + 6), 31),
                    (3, 3) => (mix(3, 5), 31),
                    _ => (0, 0),
                }
            }
            6 => {
                // A5I3
                let pixel = u32::from(tex(vramaddr + t * width + s));
                let color = pal((texpal << 4) + ((pixel & 7) << 1));
                (color, pixel >> 3)
            }
            7 => {
                // Direct color
                let color = tex16(vramaddr + ((t * width + s) << 1));
                (color, if color & 0x8000 != 0 { 31 } else { 0 }) // direct color: bit 15 = alpha
            }
            _ => (0, 31),
        }
    }
}
