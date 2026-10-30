//! Per-pixel work: depth test variants and shading (modulate, decal, toon,
//! highlight) of a polygon pixel (melonDS `RenderPixel`).
//! See GBATEK "DS 3D Polygon Attributes" and "DS 3D Toon, Edge, Fog".
//!
//! ```text
//!   depth test (POLYGON_ATTR bit 14):
//!     LESS   draw when new depth <  stored depth   (normal)
//!     EQUAL  draw when |new - stored| <= 200h (Z) / FFh (W)
//!            (decals drawn on top of an already drawn surface)
//!
//!   shading (POLYGON_ATTR bits 4-5); V = vertex color, T = texel, 6-bit RGB:
//!     0 modulate   out = ((T+1) * (V+1) - 1) / 64     texture tinted by V
//!                  alpha = texel alpha * polygon alpha
//!     1 decal      out = T*Ta + V*(31-Ta)  / 32       texture pasted on V
//!                  alpha = polygon alpha
//!     2 toon       V = TOON_TABLE[V.red], then modulate      (DISP3DCNT bit 1 = 0)
//!       highlight  V = gray(V.red), modulate, + TOON_TABLE[V.red] (bit 1 = 1)
//!     3 shadow     see band.rs
//!
//!   toon shading: V.red acts as a brightness index into a 32-color table,
//!   turning smooth Gouraud shading into flat bands:
//!     V.red:   0 ............................ 31
//!     color:   [dark   ][  mid      ][ light  ]
//! ```
//!
//! GBATEK: [DS 3D Texture Blending](https://problemkaputt.de/gbatek.htm#ds3dtextureblending),
//! [DS 3D Polygon Attributes](https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes)
use super::{Frame, expand6};
use crate::engine_3d::state::Polygon;

#[derive(Clone, Copy)]
pub(super) enum DepthTest {
    EqualZ,
    EqualW,
    Less,
    LessFrontFacing,
}

impl DepthTest {
    /// Depth test of a polygon: equal (Z or W) when POLYGON_ATTR bit 14 is set, otherwise less.
    pub(super) fn for_poly(p: &Polygon) -> Self {
        // DS 3D Polygon Attributes: https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes
        if p.attr & (1 << 14) != 0 {
            // POLYGON_ATTR bit 14: depth test EQUAL (else LESS)
            if p.w_buffer { Self::EqualW } else { Self::EqualZ }
        } else if p.facing_view {
            Self::LessFrontFacing
        } else {
            Self::Less
        }
    }

    /// Runs the depth test of a new pixel against the destination.
    #[inline]
    pub(super) fn test(self, dstz: i32, z: i32, dstattr: u32) -> bool {
        match self {
            // DS 3D Polygon Attributes: https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes
            Self::EqualZ => (dstz.wrapping_sub(z).wrapping_add(0x200) as u32) <= 0x400, // depth test EQUAL: Z-buffer tolerance +-200h
            Self::EqualW => (dstz.wrapping_sub(z).wrapping_add(0xFF) as u32) <= 0x1FE, // depth test EQUAL: W-buffer tolerance +-FFh
            Self::Less => z < dstz,
            Self::LessFrontFacing => {
                // Opaque, back-facing destination: <= instead of <.
                if dstattr & 0x0040_0010 == 0x10 { z <= dstz } else { z < dstz }
            }
        }
    }
}

impl Frame<'_> {
    /// Shades one pixel (melonDS `RenderPixel`).
    pub(super) fn render_pixel(
        &self,
        p: &Polygon,
        mut vr: u32,
        mut vg: u32,
        mut vb: u32,
        s: i16,
        t: i16,
    ) -> u32 {
        let st = self.st;
        let blend = (p.attr >> 4) & 3;
        let poly_alpha = (p.attr >> 16) & 0x1F; // POLYGON_ATTR alpha (bits 16-20)
        let wireframe = poly_alpha == 0;
        // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
        let highlight = st.disp_cnt & (1 << 1) != 0; // DISP3DCNT bit 1: highlight shading (else toon)

        if blend == 2 {
            if highlight {
                vg = vr;
                vb = vr;
            } else {
                (vr, vg, vb) = expand6(u32::from(st.toon_table[(vr >> 1) as usize & 31]));
            }
        }

        let (mut r, mut g, mut b, mut a);
        if st.disp_cnt & 1 != 0 && (p.tex_param >> 26) & 7 != 0 {
            let (tcolor, talpha) = self.texture_lookup(p.tex_param, p.tex_palette, s, t);
            let (tr, tg, tb) = expand6(tcolor);
            if blend & 1 != 0 {
                // Decal
                (r, g, b) = match talpha {
                    0 => (vr, vg, vb),
                    31 => (tr, tg, tb),
                    _ => (
                        (tr * talpha + vr * (31 - talpha)) >> 5,
                        (tg * talpha + vg * (31 - talpha)) >> 5,
                        (tb * talpha + vb * (31 - talpha)) >> 5,
                    ),
                };
                a = poly_alpha;
            } else {
                // Modulate
                r = ((tr + 1) * (vr + 1) - 1) >> 6;
                g = ((tg + 1) * (vg + 1) - 1) >> 6;
                b = ((tb + 1) * (vb + 1) - 1) >> 6;
                a = ((talpha + 1) * (poly_alpha + 1) - 1) >> 5;
            }
        } else {
            (r, g, b, a) = (vr, vg, vb, poly_alpha);
        }

        if blend == 2 && highlight {
            let (hr, hg, hb) = expand6(u32::from(st.toon_table[(vr >> 1) as usize & 31]));
            r = (r + hr).min(63);
            g = (g + hg).min(63);
            b = (b + hb).min(63);
        }
        if wireframe {
            a = 31;
        }
        r | (g << 8) | (b << 16) | (a << 24) // pack 6-bit RGB and alpha
    }
}
