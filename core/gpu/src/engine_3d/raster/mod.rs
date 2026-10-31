// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! Software 3D rasterizer.
//!
//! A port of the melonDS software renderer (`GPU3D_Soft.cpp`,
//! GPL-3.0-or-later), which reproduces the hardware's edge slopes,
//! perspective-correct interpolation, depth tests, polygon ID rules for
//! translucency and shadows, edge marking, fog and anti-aliasing.
//!
//! The whole frame is rendered at once (after the buffer swap) into a
//! 256x192 color buffer that the 2D engine A reads as BG0. Colors are
//! stored as `r | g << 8 | b << 16 | a << 24` (6-bit RGB, 5-bit alpha).
//! Each buffer has a second layer holding the pixel below an edge, used
//! by anti-aliasing and edge marking.
//!
//! A raster can also work at N times the resolution ([`Raster::with_scale`],
//! used by [`hires`] for display only): vertex positions then come from the
//! sub-pixel `hires_position`, and every size below scales with N.
//!
//! ## How one 3D frame is drawn
//!
//! ```text
//!   buffers: (256+2) x (192+2), a one-pixel border all around (edge marking
//!   looks at neighbours)
//!     layer 0:  color | depth | attr     the visible pixel
//!     layer 1:  color | depth | attr     the pixel underneath, kept at polygon
//!                                        edges for anti-aliasing
//!     attr: polygon IDs, edge flags, AA coverage, fog flag
//!
//!   1. clear      every pixel = CLEAR_COLOR / CLEAR_DEPTH
//!                 (or the rear-plane bitmaps in texture slots 2/3 when
//!                  DISP3DCNT bit 14 is set)
//!   2. polygons   in render order (opaque Y-sorted, then translucent);
//!                 each polygon filled row by row (band.rs), every pixel:
//!                 depth test -> texture + shading -> alpha test -> write/blend
//!   3. final pass per line:
//!                 edge marking   edge pixel next to a farther pixel with a
//!                                different polygon ID -> EDGE_COLOR[ID / 8]
//!                 fog            color += (FOG_COLOR - color) * density / 128,
//!                                density from FOG_TABLE indexed by depth
//!                 anti-aliasing  edge pixel = mix(layer 0, layer 1, coverage)
//!
//!   painter's view of step 2 (opaque first, translucent last):
//!      clear      opaque A        opaque B (nearer)      translucent C
//!     +------+   +------+        +------+               +------+
//!     |      |   | AAA  |        | AAA  |               | AAA  |
//!     |      |-->| AAA  |  -->   | ABBB |      -->      | ABcc |  c = C blended
//!     |      |   |      |        |  BBB |               |  Bcc |  over what is
//!     +------+   +------+        +------+               +------+  below
//!
//!   the frame is split into horizontal bands rendered in parallel:
//!     +--------------------+ y = 0
//!     | band 0   thread 0  |
//!     +--------------------+
//!     | band 1   thread 1  |
//!     +--------------------+
//!     | ...                |
//!     +--------------------+ y = 191
//!     the final pass of the lines at band borders runs after all bands
//!     finish, because it reads the lines above and below.
//! ```
//!
//! GBATEK: [DS 3D Toon, Edge, Fog, Alpha-Blending, Anti-Aliasing](https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing),
//! [DS 3D Rear-Plane](https://problemkaputt.de/gbatek.htm#ds3drearplane)

mod band;
pub mod hires;
mod pixel;
mod slope;
mod texture;

use band::Band;
use rayon::prelude::*;

use crate::{
    engine_3d::state::{Polygon, RenderState, Vertex},
    vram::{Region, Vram},
};

const TEX_SIZE: usize = 512 * 1024;
const PAL_SIZE: usize = 128 * 1024;

/// Buffer geometry at a resolution scale: the visible `width` x `height`
/// area with a one-pixel border all around (for edge marking).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Dims {
    scale: usize,
    width: usize,
    height: usize,
    /// Row pitch (`width + 2`).
    pitch: usize,
    /// Size of one buffer layer.
    layer: usize,
    /// Index of the visible pixel (0, 0).
    first: usize,
}

impl Dims {
    const fn new(scale: usize) -> Self {
        let (width, height) = (256 * scale, 192 * scale);
        let pitch = width + 2;
        Self { scale, width, height, pitch, layer: pitch * (height + 2), first: pitch + 1 }
    }
}

/// Expands a BGR555 color to the 6-bit components used by the renderer.
#[inline]
fn expand6(c: u32) -> (u32, u32, u32) {
    let e = |v: u32| if v != 0 { v + 1 } else { 0 };
    // DS 3D Toon, Edge, Fog, Alpha-Blending, Anti-Aliasing: https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing
    (e((c << 1) & 0x3E), e((c >> 4) & 0x3E), e((c >> 9) & 0x3E)) // BGR555 components widened to 6 bits
}

/// Inputs of one frame.
struct Frame<'a> {
    st: &'a RenderState,
    polys: &'a [Polygon],
    verts: &'a [Vertex],
    order: &'a [usize],
    /// Flat copies of texture image and palette VRAM.
    tex: &'a [u8],
    pal: &'a [u8],
    /// Visible width in pixels (256 x scale).
    width: i32,
}

impl Frame<'_> {
    /// Vertex `i` of polygon `p`.
    #[inline]
    fn vtx(&self, p: &Polygon, i: usize) -> &Vertex {
        &self.verts[p.vertices[i]]
    }
}

/// Rasterizer buffers.
#[derive(Debug)]
pub struct Raster {
    dims: Dims,
    color: Vec<u32>,
    depth: Vec<i32>,
    attr: Vec<u32>,
    /// Per-thread scanline bands (reused between frames).
    bands: Vec<Band>,
    tex: Vec<u8>,
    pal: Vec<u8>,
    /// Scratch buffers for the next frame's texture data.
    tex_next: Vec<u8>,
    pal_next: Vec<u8>,
    /// Whether the buffers hold a rendered frame.
    valid: bool,
    /// Counts the frames actually rendered (a new count means new pixels).
    generation: u64,
}

impl Default for Raster {
    fn default() -> Self {
        Self::with_scale(1)
    }
}

impl Raster {
    /// A raster working at `scale` times the DS resolution (1 = native).
    pub fn with_scale(scale: usize) -> Self {
        let dims = Dims::new(scale.max(1));
        // Only the native raster snapshots VRAM; scaled ones borrow it.
        let vram_size = |n: usize| if dims.scale == 1 { n } else { 0 };
        Self {
            dims,
            color: vec![0; dims.layer * 2],
            depth: vec![0; dims.layer * 2],
            attr: vec![0; dims.layer * 2],
            bands: Vec::new(),
            tex: vec![0; vram_size(TEX_SIZE)],
            pal: vec![0; vram_size(PAL_SIZE)],
            tex_next: vec![0; vram_size(TEX_SIZE)],
            pal_next: vec![0; vram_size(PAL_SIZE)],
            valid: false,
            generation: 0,
        }
    }

    /// The resolution scale.
    pub const fn scale(&self) -> usize {
        self.dims.scale
    }

    /// Frames rendered so far.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// One rendered line (256 x scale pixels, layer 0).
    pub fn line(&self, y: usize) -> &[u32] {
        let d = self.dims;
        let start = d.first + y.min(d.height - 1) * d.pitch;
        &self.color[start..start + d.width]
    }

    /// Renders a frame. `order` lists the polygons to draw (indices into
    /// `polys`), sorted as the hardware does. When `changed` is false
    /// (same polygons and registers as the last frame) and the texture
    /// data is unchanged too, the previous frame is kept.
    ///
    /// ```text
    /// from: Gpu::render_3d_frame()
    ///   |
    ///   v
    /// render(state, polys, order, verts, vram, changed)
    ///   1. snapshot texture image / palette VRAM; changed?
    ///   2. nothing changed -> keep the last frame
    ///   3. render_frame():
    ///        bands in parallel    clear_rows() (color/depth/attr or the
    ///                             rear plane), Band::render: polygon
    ///                             edges (slope.rs), spans, texturing
    ///                             (texture.rs), depth test, blending
    ///                             (pixel.rs), then the band's final pass
    ///        border lines         edge marking, fog, anti-aliasing of the
    ///                             first / last line of each band
    ///   |
    ///   v
    /// to:   line(y) is read by render_bg_3d() while composing BG0
    /// ```
    pub fn render(
        &mut self,
        st: &RenderState,
        polys: &[Polygon],
        order: &[usize],
        verts: &[Vertex],
        vram: &Vram,
        changed: bool,
    ) {
        vram.copy_region(Region::TexImage, &mut self.tex_next);
        vram.copy_region(Region::TexPal, &mut self.pal_next);
        let textures_changed = self.tex_next != self.tex || self.pal_next != self.pal;
        if textures_changed {
            std::mem::swap(&mut self.tex, &mut self.tex_next);
            std::mem::swap(&mut self.pal, &mut self.pal_next);
        }
        if self.valid && !changed && !textures_changed {
            return;
        }
        self.valid = true;
        self.generation += 1;

        let (tex, pal) = (std::mem::take(&mut self.tex), std::mem::take(&mut self.pal));
        let f =
            Frame { st, polys, verts, order, tex: &tex, pal: &pal, width: self.dims.width as i32 };
        self.render_frame(&f);
        self.tex = tex;
        self.pal = pal;
    }

    /// Texture image VRAM as last snapshotted (native raster).
    pub(super) fn tex(&self) -> &[u8] {
        &self.tex
    }

    /// Texture palette VRAM as last snapshotted (native raster).
    pub(super) fn pal(&self) -> &[u8] {
        &self.pal
    }

    /// Renders a frame with texture data from elsewhere (a scaled raster
    /// borrows the native one's VRAM snapshot).
    pub(super) fn render_with(
        &mut self,
        st: &RenderState,
        polys: &[Polygon],
        order: &[usize],
        verts: &[Vertex],
        tex: &[u8],
        pal: &[u8],
    ) {
        self.valid = true;
        self.generation += 1;
        let f = Frame { st, polys, verts, order, tex, pal, width: self.dims.width as i32 };
        self.render_frame(&f);
    }

    /// Clears the border rows, renders the bands in parallel, then the
    /// final pass of the lines next to band borders.
    fn render_frame(&mut self, f: &Frame) {
        let d = self.dims;
        self.clear_borders(f.st);

        // Scanline bands rendered in parallel. Each band owns a copy of its
        // rows (cleared, rendered and finished there, then written back to
        // its own part of the buffers); polygon edges are set up directly at
        // the band's first line. Scaled rasters use more bands than threads
        // so that bands crowded with polygons do not hold the others up.
        let threads = crate::threads::render_threads();
        let count = if d.scale == 1 { threads.clamp(1, 8) } else { (threads * 2).clamp(1, 32) };
        let lines = d.height.div_ceil(count);
        self.bands.resize_with(count, Band::default);
        let rows: Vec<_> = band_rows(&mut self.color, d, count, lines)
            .into_iter()
            .zip(band_rows(&mut self.depth, d, count, lines))
            .zip(band_rows(&mut self.attr, d, count, lines))
            .collect();
        type Rows<'a> = (([&'a mut [u32]; 2], [&'a mut [i32]; 2]), [&'a mut [u32]; 2]);
        let job =
            |(i, (band, ((mut color, mut depth), mut attr))): (usize, (&mut Band, Rows<'_>))| {
                let y0 = (i * lines).min(d.height);
                let y1 = ((i + 1) * lines).min(d.height);
                band.load(d, f, color[1], depth[1], attr[1], y0, y1);
                band.render(f);
                band.final_pass(f.st);
                band.store(&mut color, &mut depth, &mut attr);
            };
        if crate::threads::parallel() {
            let bands = &mut self.bands;
            crate::threads::run(|| bands.par_iter_mut().zip(rows).enumerate().for_each(job));
        } else {
            self.bands.iter_mut().zip(rows).enumerate().for_each(job);
        }

        // Edge marking reads the lines above and below: the first and last
        // line of each band are finished here, with all bands in place.
        for i in 0..self.bands.len() {
            let (y0, y1) = (self.bands[i].y0, self.bands[i].y1);
            if y1 > y0 {
                self.scanline_final_pass(f.st, y0);
                if y1 - 1 != y0 {
                    self.scanline_final_pass(f.st, y1 - 1);
                }
            }
        }
    }

    /// Clears the rows above and below the visible area of layer 0 (read by
    /// edge marking); the visible rows are cleared by the bands.
    fn clear_borders(&mut self, st: &RenderState) {
        let d = self.dims;
        // DS 3D Rear-Plane: https://problemkaputt.de/gbatek.htm#ds3drearplane
        let clear_z = ((st.clear_attr2 & 0x7FFF) * 0x200 + 0x1FF) as i32; // CLEAR_DEPTH (bits 0-14) to 24-bit depth: value * 200h + 1FFh
        let poly_id = st.clear_attr1 & 0x3F00_0000; // CLEAR_COLOR polygon ID (bits 24-29)
        for range in [0..d.pitch, d.pitch * (d.height + 1)..d.layer] {
            self.color[range.clone()].fill(0);
            self.depth[range.clone()].fill(clear_z);
            self.attr[range].fill(poly_id);
        }
    }

    /// Blends a translucent pixel over the destination (when DISP3DCNT bit 3 enables blending).
    fn alpha_blend(st: &RenderState, src: u32, dst: u32, mut alpha: u32) -> u32 {
        let mut dst_alpha = dst >> 24;
        if dst_alpha == 0 {
            return src;
        }
        // DS 3D Toon, Edge, Fog, Alpha-Blending, Anti-Aliasing: https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing
        let (mut r, mut g, mut b) = (src & 0x3F, (src >> 8) & 0x3F, (src >> 16) & 0x3F); // 6-bit RGB of the source pixel (bits 0-5, 8-13, 16-21)
        // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
        if st.disp_cnt & (1 << 3) != 0 {
            // DISP3DCNT bit 3: alpha blending enable
            let (dr, dg, db) = (dst & 0x3F, (dst >> 8) & 0x3F, (dst >> 16) & 0x3F); // 6-bit RGB of the destination pixel
            alpha += 1;
            r = (r * alpha + dr * (32 - alpha)) >> 5;
            g = (g * alpha + dg * (32 - alpha)) >> 5;
            b = (b * alpha + db * (32 - alpha)) >> 5;
            alpha -= 1;
        }
        if alpha > dst_alpha {
            dst_alpha = alpha;
        }
        r | (g << 8) | (b << 16) | (dst_alpha << 24) // pack 6-bit RGB (bits 0/8/16) and alpha (bits 24-28)
    }

    /// Edge marking, fog and anti-aliasing of one finished line.
    fn scanline_final_pass(&mut self, st: &RenderState, y: usize) {
        let d = self.dims;
        let mut px = Pixels {
            color: &mut self.color,
            depth: &self.depth,
            attr: &mut self.attr,
            pitch: d.pitch,
            layer: d.layer,
        };
        px.final_pass(st, d.first + y * d.pitch, d.width);
    }
}

/// Clears whole rows (`pitch` long, from the left border) of layer 0,
/// starting at line `y0`: to CLEAR_COLOR/DEPTH, or the rear-plane bitmap.
fn clear_rows(
    f: &Frame,
    d: Dims,
    color: &mut [u32],
    depth: &mut [i32],
    attr: &mut [u32],
    y0: usize,
) {
    let st = f.st;
    // DS 3D Rear-Plane: https://problemkaputt.de/gbatek.htm#ds3drearplane
    let clear_z = ((st.clear_attr2 & 0x7FFF) * 0x200 + 0x1FF) as i32; // CLEAR_DEPTH (bits 0-14) to 24-bit depth: value * 200h + 1FFh
    let poly_id = st.clear_attr1 & 0x3F00_0000; // CLEAR_COLOR polygon ID (bits 24-29)

    // Borders (outside the visible area) only matter for edge marking.
    color.fill(0);
    depth.fill(clear_z);
    attr.fill(poly_id);
    let rows = color.len() / d.pitch;

    if st.disp_cnt & (1 << 14) != 0 {
        // Rear-plane bitmap from texture slots 2 (color) and 3 (depth),
        // one DS pixel per scale x scale block.
        let yoff0 = ((st.clear_attr2 >> 24) & 0xFF) as u8;
        let xoff0 = ((st.clear_attr2 >> 16) & 0xFF) as u8; // CLRIMAGE_OFFSET X scroll (bits 16-23 of the packed register)
        for row in 0..rows {
            let yoff = yoff0.wrapping_add(((y0 + row) / d.scale) as u8);
            for x in 0..d.width {
                let xoff = xoff0.wrapping_add((x / d.scale) as u8);
                let ofs = (u32::from(yoff) << 9) + (u32::from(xoff) << 1);
                let rd = |a: u32| {
                    let a = a as usize;
                    u32::from(u16::from_le_bytes([f.tex[a], f.tex[a + 1]]))
                };
                let val2 = rd(0x40000 + ofs); // rear-plane color bitmap in texture slot 2
                let val3 = rd(0x60000 + ofs); // rear-plane depth bitmap in texture slot 3
                let (r, g, b) = expand6(val2);
                let a = if val2 & 0x8000 != 0 { 0x1F00_0000 } else { 0 }; // color bitmap bit 15: alpha (opaque/transparent)
                let addr = row * d.pitch + 1 + x;
                color[addr] = r | (g << 8) | (b << 16) | a; // pack 6-bit RGB with alpha
                depth[addr] = ((val3 & 0x7FFF) * 0x200 + 0x1FF) as i32; // depth bitmap bits 0-14 to 24-bit depth
                attr[addr] = poly_id | (val3 & 0x8000); // depth bitmap bit 15: fog enable
            }
        }
    } else {
        let (r, g, b) = expand6(st.clear_attr1);
        let a = (st.clear_attr1 >> 16) & 0x1F; // CLEAR_COLOR alpha (bits 16-20)
        let clear_color = r | (g << 8) | (b << 16) | (a << 24); // pack 6-bit RGB and alpha (bits 24-28)
        let clear_attr = poly_id | (st.clear_attr1 & 0x8000); // CLEAR_COLOR fog enable (bit 15)
        for row in 0..rows {
            let start = row * d.pitch + 1;
            color[start..start + d.width].fill(clear_color);
            attr[start..start + d.width].fill(clear_attr);
        }
    }
}

/// Lines `i * lines ..` of both layers of `buf` (whole rows, borders
/// included), one pair of slices per band.
fn band_rows<T>(buf: &mut [T], d: Dims, count: usize, lines: usize) -> Vec<[&mut [T]; 2]> {
    fn split<T>(layer: &mut [T], d: Dims, count: usize, lines: usize) -> Vec<&mut [T]> {
        let mut rest = &mut layer[d.pitch..d.pitch * (d.height + 1)];
        (0..count)
            .map(|i| {
                let rows = lines.min(d.height.saturating_sub(i * lines));
                let (band, tail) = std::mem::take(&mut rest).split_at_mut(rows * d.pitch);
                rest = tail;
                band
            })
            .collect()
    }
    let (l0, l1) = buf.split_at_mut(d.layer);
    split(l0, d, count, lines)
        .into_iter()
        .zip(split(l1, d, count, lines))
        .map(|(a, b)| [a, b])
        .collect()
}

/// Buffers of whole rows (both layers) that the final pass works on: the
/// full frame or one band's copy.
struct Pixels<'a> {
    color: &'a mut [u32],
    pub(super) depth: &'a [i32],
    pub(super) attr: &'a mut [u32],
    pub(super) pitch: usize,
    /// Offset of the second layer.
    pub(super) layer: usize,
}

impl Pixels<'_> {
    /// Fog density for the pixel at `addr` from its depth and the fog table.
    fn fog_density(&self, st: &RenderState, addr: usize) -> u32 {
        let z = self.depth[addr] as u32;
        let (id, frac) = if z < st.fog_offset {
            (0, 0)
        } else {
            let z = ((z - st.fog_offset) >> 2) << st.fog_shift;
            let id = (z >> 17) as usize;
            // DS 3D Toon, Edge, Fog, Alpha-Blending, Anti-Aliasing: https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing
            if id >= 32 { (32, 0) } else { (id, z & 0x1FFFF) } // fog table index and 17-bit fraction between entries
        };
        let d = (u32::from(st.fog_density[id]) * (0x20000 - frac) // linear interpolation between two FOG_TABLE entries
            + u32::from(st.fog_density[id + 1]) * frac)
            >> 17;
        if d >= 127 { 128 } else { d }
    }

    /// Mixes the fog color into the pixel at `addr` (pixels with the fog attribute bit only).
    fn apply_fog(&mut self, st: &RenderState, addr: usize) {
        // DS 3D Toon, Edge, Fog, Alpha-Blending, Anti-Aliasing: https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing
        if self.attr[addr] & (1 << 15) == 0 {
            // bit 15: fog enable for this pixel
            return;
        }
        let density = self.fog_density(st, addr);
        let src = self.color[addr];
        let (mut r, mut g, mut b) = (src & 0x3F, (src >> 8) & 0x3F, (src >> 16) & 0x3F); // 6-bit RGB of the pixel
        let mut a = (src >> 24) & 0x1F; // 5-bit alpha (bits 24-28)
        if st.disp_cnt & (1 << 6) == 0 {
            // DISP3DCNT bit 6: fog mode (0 = color and alpha, 1 = alpha only)
            let (fr, fg, fb) = expand6(st.fog_color);
            r = (fr * density + r * (128 - density)) >> 7;
            g = (fg * density + g * (128 - density)) >> 7;
            b = (fb * density + b * (128 - density)) >> 7;
        }
        let fa = (st.fog_color >> 16) & 0x1F; // FOG_COLOR alpha (bits 16-20)
        a = (fa * density + a * (128 - density)) >> 7;
        self.color[addr] = r | (g << 8) | (b << 16) | (a << 24); // pack 6-bit RGB and alpha
    }

    /// Edge marking, fog and anti-aliasing of the `width` pixels from
    /// `row` (the lines above and below must be in the buffers too).
    pub(super) fn final_pass(&mut self, st: &RenderState, row: usize, width: usize) {
        let (pitch, layer) = (self.pitch, self.layer);

        if st.disp_cnt & (1 << 5) != 0 {
            // DISP3DCNT bit 5: edge marking
            for x in 0..width {
                let addr = row + x;
                let attr = self.attr[addr];
                if attr & 0xF == 0 {
                    // edge flags (bits 0-3) clear: not an edge pixel
                    continue;
                }
                let id = attr >> 24;
                let z = self.depth[addr];
                let differs = |a: usize| id != self.attr[a] >> 24 && z < self.depth[a];
                if differs(addr - 1)
                    || differs(addr + 1)
                    || differs(addr - pitch)
                    || differs(addr + pitch)
                {
                    let (r, g, b) = expand6(u32::from(st.edge_table[(id >> 3) as usize & 7]));
                    self.color[addr] = r | (g << 8) | (b << 16) | (self.color[addr] & 0xFF00_0000); // edge color replaces RGB, alpha is kept
                    self.attr[addr] = (self.attr[addr] & 0xFFFF_E0FF) | 0x1000; // clear coverage (bits 8-12) and set it to half (10h)
                }
            }
        }

        if st.disp_cnt & (1 << 7) != 0 {
            // DISP3DCNT bit 7: fog enable
            for x in 0..width {
                let addr = row + x;
                let attr = self.attr[addr];
                self.apply_fog(st, addr);
                if attr & 0xF != 0 {
                    // edge flags (bits 0-3)
                    self.apply_fog(st, addr + layer);
                }
            }
        }

        if st.disp_cnt & (1 << 4) != 0 {
            // DISP3DCNT bit 4: anti-aliasing
            for x in 0..width {
                let addr = row + x;
                let attr = self.attr[addr];
                if attr & 0xF == 0 {
                    // edge flags (bits 0-3)
                    continue;
                }
                let mut coverage = (attr >> 8) & 0x1F; // anti-aliasing coverage (bits 8-12)
                if coverage == 0x1F {
                    // full coverage
                    continue;
                }
                let bot = self.color[addr + layer];
                if coverage == 0 {
                    self.color[addr] = bot;
                    continue;
                }
                let top = self.color[addr];
                let ch = |c: u32, s: u32| (c >> s) & 0x3F; // 6-bit color channel
                let (mut r, mut g, mut b) = (ch(top, 0), ch(top, 8), ch(top, 16));
                let mut a = (top >> 24) & 0x1F; // 5-bit alpha (bits 24-28)
                let bot_a = (bot >> 24) & 0x1F; // 5-bit alpha (bits 24-28)
                coverage += 1;
                if bot_a > 0 {
                    r = (r * coverage + ch(bot, 0) * (32 - coverage)) >> 5;
                    g = (g * coverage + ch(bot, 8) * (32 - coverage)) >> 5;
                    b = (b * coverage + ch(bot, 16) * (32 - coverage)) >> 5;
                }
                a = (a * coverage + bot_a * (32 - coverage)) >> 5;
                self.color[addr] = r | (g << 8) | (b << 16) | (a << 24); // pack 6-bit RGB and alpha
            }
        }
    }
}
