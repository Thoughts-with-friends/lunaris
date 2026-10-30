//! Scanline rasterization of one horizontal band of the frame: polygon
//! edge setup, span fill rules, shadow masks and translucent pixels.
//! Bands are rendered in parallel (see [`super::Raster::render`]).
//!
//! ## Filling a polygon, one scanline at a time
//!
//! ```text
//!   a polygon is walked down from its top vertex; a left and a right edge
//!   are followed, each moving on to the next vertex when it is passed:
//!
//!                  v0 (top)
//!                 /  \
//!    left edge   /    \   right edge
//!    v0 -> v3   /      \  v0 -> v1
//!              /        \
//!         v3  *          * v1
//!              \  ====== |      <- row y: pixels from the left edge X
//!    then       \        |         to the right edge X are filled
//!    v3 -> v2    \       |  then v1 -> v2
//!                 \      |
//!                  *-----*
//!                 v2      (bottom)
//!
//!   one row (span) has three segments:
//!      |<- left edge ->|<-------- interior -------->|<- right edge ->|
//!      edge segments: edge flags (for edge marking) and AA coverage
//!      interior: filled normally
//!      a pixel belongs to the span when  left X <= x < right X
//!      wireframe polygons (alpha 0) draw only the edge segments
//!
//!   per pixel: depth test -> texture / shading (pixel.rs)
//!              -> alpha test (ALPHA_TEST_REF, DISP3DCNT bit 2)
//!              -> opaque: write color + depth + polygon ID
//!                 translucent: blend over the pixel below, skipped where a
//!                 translucent pixel with the same polygon ID was already
//!                 drawn (no double blending inside one object)
//!
//!   shadows (polygon mode 3):
//!      shadow mask (ID 0):  marks a stencil where its depth test FAILS
//!                           (i.e. where the shadow volume is behind something)
//!      shadow (ID != 0):    drawn only on stencil-marked pixels whose
//!                           polygon ID differs from the shadow's
//! ```
//!
//! GBATEK: [DS 3D Polygon Attributes](https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes),
//! [DS 3D Shadow Polygons](https://problemkaputt.de/gbatek.htm#ds3dshadowpolygons)
use super::{
    Dims, Frame, Raster,
    pixel::DepthTest,
    slope::{Interp, Slope},
};
use crate::engine_3d::state::{Polygon, RenderState};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RendererPolygon {
    pub(super) poly: usize,
    pub(super) slope_l: Slope<false>,
    pub(super) slope_r: Slope<true>,
    pub(super) xl: i32,
    pub(super) xr: i32,
    pub(super) cur_vl: usize,
    pub(super) cur_vr: usize,
    pub(super) next_vl: usize,
    pub(super) next_vr: usize,
}

/// Per-scanline span parameters shared by the three span segments.
pub(super) struct Span {
    pub(super) xstart: i32,
    pub(super) xend: i32,
    pub(super) l_edgelen: i32,
    pub(super) r_edgelen: i32,
    pub(super) l_edgecov: u32,
    pub(super) r_edgecov: u32,
    pub(super) l_filledge: bool,
    pub(super) r_filledge: bool,
    pub(super) zl: i32,
    pub(super) zr: i32,
    pub(super) interp_x: Interp<false>,
    pub(super) yedge: u32,
    /// Visible width: pixels at or past it are not drawn.
    pub(super) width: i32,
}

/// A horizontal band of the frame rendered by one thread: its rows of both
/// buffer layers (each row keeps the one-pixel border of the full buffers)
/// and its own shadow stencil.
#[derive(Debug, Default)]
pub(super) struct Band {
    pub(super) color: Vec<u32>,
    pub(super) depth: Vec<i32>,
    pub(super) attr: Vec<u32>,
    pub(super) y0: usize,
    pub(super) y1: usize,
    /// Row pitch of the full buffers.
    pub(super) pitch: usize,
    /// Visible width (stencil row length).
    pub(super) width: usize,
    /// Size of one layer of this band.
    pub(super) layer: usize,
    pub(super) stencil: Vec<u8>,
    pub(super) prev_is_shadow_mask: bool,
    pub(super) polys: Vec<RendererPolygon>,
}

impl Band {
    /// Takes rows `y0..y1`: layer 0 cleared for a new frame, layer 1
    /// copied from the frame buffers (`*1`: this band's rows of layer 1).
    #[expect(clippy::too_many_arguments, reason = "one call site")]
    pub(super) fn load(
        &mut self,
        d: Dims,
        f: &Frame,
        color1: &[u32],
        depth1: &[i32],
        attr1: &[u32],
        y0: usize,
        y1: usize,
    ) {
        self.y0 = y0;
        self.y1 = y1;
        self.pitch = d.pitch;
        self.width = d.width;
        self.layer = (y1 - y0) * d.pitch;
        let n = self.layer * 2;
        self.color.resize(n, 0);
        self.depth.resize(n, 0);
        self.attr.resize(n, 0);
        self.stencil.clear();
        self.stencil.resize(d.width * 2, 0);
        self.prev_is_shadow_mask = false;
        let l = self.layer;
        super::clear_rows(
            f,
            d,
            &mut self.color[..l],
            &mut self.depth[..l],
            &mut self.attr[..l],
            y0,
        );
        self.color[l..].copy_from_slice(color1);
        self.depth[l..].copy_from_slice(depth1);
        self.attr[l..].copy_from_slice(attr1);
    }

    /// Writes the band's rows back (`[layer 0, layer 1]` rows of the frame
    /// buffers).
    pub(super) fn store(
        &self,
        color: &mut [&mut [u32]; 2],
        depth: &mut [&mut [i32]; 2],
        attr: &mut [&mut [u32]; 2],
    ) {
        let l = self.layer;
        for k in 0..2 {
            color[k].copy_from_slice(&self.color[k * l..(k + 1) * l]);
            depth[k].copy_from_slice(&self.depth[k * l..(k + 1) * l]);
            attr[k].copy_from_slice(&self.attr[k * l..(k + 1) * l]);
        }
    }

    /// Edge marking, fog and anti-aliasing of the band's lines that have
    /// both neighbours in the band (the others are finished by the raster).
    pub(super) fn final_pass(&mut self, st: &RenderState) {
        let mut px = super::Pixels {
            color: &mut self.color,
            depth: &self.depth,
            attr: &mut self.attr,
            pitch: self.pitch,
            layer: self.layer,
        };
        for y in self.y0 + 1..self.y1.saturating_sub(1) {
            px.final_pass(st, (y - self.y0) * self.pitch + 1, self.width);
        }
    }

    /// Index of pixel 0 of line `y` (layer 0).
    #[inline]
    pub(super) fn row(&self, y: i32) -> usize {
        (y as usize - self.y0) * self.pitch + 1
    }

    /// Renders this band's rows (Raster::render_frame, one thread per band):
    /// collect the polygons overlapping the band, set their edges up at the
    /// first row, then render_scanline() for every row.
    pub(super) fn render(&mut self, f: &Frame) {
        let (y0, y1) = (self.y0 as i32, self.y1 as i32);
        self.polys.clear();
        for &i in f.order {
            let p = &f.polys[i];
            if p.degenerate || p.y_top >= y1 {
                continue;
            }
            // Same visibility rule as `render_scanline`.
            let last = if p.y_bottom == p.y_top { p.y_top } else { p.y_bottom - 1 };
            if last < y0 {
                continue;
            }
            let mut rp = RendererPolygon { poly: i, ..RendererPolygon::default() };
            Self::setup_polygon(f, &mut rp);
            if p.y_top < y0 && p.y_top != p.y_bottom {
                Self::setup_left_edge(f, &mut rp, y0);
                Self::setup_right_edge(f, &mut rp, y0);
            }
            self.polys.push(rp);
        }
        for y in y0..y1 {
            self.render_scanline(f, y);
        }
    }

    /// Next vertex index around the polygon in the given winding.
    pub(super) fn next_vertex(p: &Polygon, cur: usize, forward: bool) -> usize {
        let n = p.num_vertices;
        if forward {
            if cur + 1 >= n { 0 } else { cur + 1 }
        } else if cur == 0 {
            n - 1
        } else {
            cur - 1
        }
    }

    /// Advances the left edge to the vertex pair spanning row `y`.
    pub(super) fn setup_left_edge(f: &Frame, rp: &mut RendererPolygon, y: i32) {
        let p = &f.polys[rp.poly];
        while y >= f.vtx(p, rp.next_vl).final_position[1] && rp.cur_vl != p.v_bottom {
            rp.cur_vl = rp.next_vl;
            rp.next_vl = Self::next_vertex(p, rp.cur_vl, p.facing_view);
        }
        let (a, b) = (f.vtx(p, rp.cur_vl), f.vtx(p, rp.next_vl));
        rp.xl = rp.slope_l.setup(
            a.final_position[0],
            b.final_position[0],
            a.final_position[1],
            b.final_position[1],
            p.final_w[rp.cur_vl],
            p.final_w[rp.next_vl],
            y,
            p.w_buffer,
        );
    }

    /// Advances the right edge to the vertex pair spanning row `y`.
    pub(super) fn setup_right_edge(f: &Frame, rp: &mut RendererPolygon, y: i32) {
        let p = &f.polys[rp.poly];
        while y >= f.vtx(p, rp.next_vr).final_position[1] && rp.cur_vr != p.v_bottom {
            rp.cur_vr = rp.next_vr;
            rp.next_vr = Self::next_vertex(p, rp.cur_vr, !p.facing_view);
        }
        let (a, b) = (f.vtx(p, rp.cur_vr), f.vtx(p, rp.next_vr));
        rp.xr = rp.slope_r.setup(
            a.final_position[0],
            b.final_position[0],
            a.final_position[1],
            b.final_position[1],
            p.final_w[rp.cur_vr],
            p.final_w[rp.next_vr],
            y,
            p.w_buffer,
        );
    }

    /// Starts walking a polygon from its top vertex (both edges).
    pub(super) fn setup_polygon(f: &Frame, rp: &mut RendererPolygon) {
        let p = &f.polys[rp.poly];
        rp.cur_vl = p.v_top;
        rp.cur_vr = p.v_top;
        rp.next_vl = Self::next_vertex(p, p.v_top, p.facing_view);
        rp.next_vr = Self::next_vertex(p, p.v_top, !p.facing_view);

        if p.y_bottom == p.y_top {
            // Flat polygon: span from the leftmost to the rightmost vertex.
            let x = |i: usize| f.vtx(p, i).final_position[0];
            let (mut vtop, mut vbot) = (0, 0);
            for i in [1, p.num_vertices - 1] {
                if x(i) < x(vtop) {
                    vtop = i;
                }
                if x(i) > x(vbot) {
                    vbot = i;
                }
            }
            rp.cur_vl = vtop;
            rp.next_vl = vtop;
            rp.cur_vr = vbot;
            rp.next_vr = vbot;
            rp.xl = rp.slope_l.setup_dummy(x(vtop), p.w_buffer);
            rp.xr = rp.slope_r.setup_dummy(x(vbot), p.w_buffer);
        } else {
            Self::setup_left_edge(f, rp, p.y_top);
            Self::setup_right_edge(f, rp, p.y_top);
        }
    }

    /// Draws row `y` of every polygon covering it, shadow masks and normal polygons in render
    /// order.
    pub(super) fn render_scanline(&mut self, f: &Frame, y: i32) {
        for i in 0..self.polys.len() {
            let mut rp = self.polys[i];
            let p = &f.polys[rp.poly];
            if y >= p.y_top && (y < p.y_bottom || (y == p.y_top && p.y_bottom == p.y_top)) {
                if p.is_shadow_mask {
                    self.render_shadow_mask_scanline(f, &mut rp, y);
                } else {
                    self.render_polygon_scanline(f, &mut rp, y);
                }
                self.polys[i] = rp;
            }
        }
    }

    /// Advances the edges to scanline `y` and computes the span (shared
    /// by polygons and shadow masks). Also returns whether the span is
    /// swapped (the right edge is left of the left edge).
    pub(super) fn setup_span(f: &Frame, rp: &mut RendererPolygon, y: i32) -> (Span, bool) {
        let p = &f.polys[rp.poly];
        if p.y_top != p.y_bottom {
            if y >= f.vtx(p, rp.next_vl).final_position[1] && rp.cur_vl != p.v_bottom {
                Self::setup_left_edge(f, rp, y);
            }
            if y >= f.vtx(p, rp.next_vr).final_position[1] && rp.cur_vr != p.v_bottom {
                Self::setup_right_edge(f, rp, y);
            }
        }

        // DS 3D Polygon Attributes: https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes
        let poly_alpha = (p.attr >> 16) & 0x1F; // POLYGON_ATTR alpha (bits 16-20)
        let wireframe = poly_alpha == 0;
        let mut xstart = rp.xl;
        let mut xend = rp.xr;
        let mut wl = rp.slope_l.interp.interpolate(p.final_w[rp.cur_vl], p.final_w[rp.next_vl]);
        let mut wr = rp.slope_r.interp.interpolate(p.final_w[rp.cur_vr], p.final_w[rp.next_vr]);
        let mut zl = rp.slope_l.interp.interpolate_z(p.final_z[rp.cur_vl], p.final_z[rp.next_vl]);
        let mut zr = rp.slope_r.interp.interpolate_z(p.final_z[rp.cur_vr], p.final_z[rp.next_vr]);

        // Right edges are exclusive, except for vertical-ish spans.
        if rp.slope_r.increment == 0 && (rp.slope_l.increment != 0 || xstart != xend) && xend != 0 {
            xend -= 1;
        }

        // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
        let fill_all = f.st.disp_cnt & ((1 << 4) | (1 << 5)) != 0 // DISP3DCNT bit 4 (anti-aliasing) or bit 5 (edge marking): fill polygon edges
            || (poly_alpha < 31 && f.st.disp_cnt & (1 << 3) != 0) // DISP3DCNT bit 3: alpha blending
            || wireframe;
        let last_line = y == p.y_bottom - 1;
        let swapped = xstart > xend;
        let (l_edgelen, l_edgecov, r_edgelen, r_edgecov, l_filledge, r_filledge);
        if swapped {
            let (vlnext, vrnext) = (f.vtx(p, rp.next_vr), f.vtx(p, rp.next_vl));
            let differ = vlnext.final_position[0] != vrnext.final_position[0];
            (l_edgelen, l_edgecov) = rp.slope_r.edge_params(true);
            (r_edgelen, r_edgecov) = rp.slope_l.edge_params(true);
            std::mem::swap(&mut xstart, &mut xend);
            std::mem::swap(&mut wl, &mut wr);
            std::mem::swap(&mut zl, &mut zr);
            let (sl, sr) = (&rp.slope_l, &rp.slope_r);
            if fill_all {
                (l_filledge, r_filledge) = (true, true);
            } else {
                l_filledge = (sr.negative || !sr.x_major) || (last_line && sr.x_major && differ);
                r_filledge = (!sl.negative && sl.x_major)
                    || (!(sl.negative && sl.x_major) && sr.increment == 0)
                    || (last_line && sl.x_major && differ);
            }
        } else {
            let (vlnext, vrnext) = (f.vtx(p, rp.next_vl), f.vtx(p, rp.next_vr));
            let differ = vlnext.final_position[0] != vrnext.final_position[0];
            (l_edgelen, l_edgecov) = rp.slope_l.edge_params(false);
            (r_edgelen, r_edgecov) = rp.slope_r.edge_params(false);
            let (sl, sr) = (&rp.slope_l, &rp.slope_r);
            if fill_all {
                (l_filledge, r_filledge) = (true, true);
            } else {
                l_filledge = ((sl.negative || !sl.x_major) || (last_line && sl.x_major && differ))
                    || (sl.increment == sr.increment && xstart + l_edgelen == xend + 1);
                r_filledge = (!sr.negative && sr.x_major)
                    || sr.increment == 0
                    || (last_line && sr.x_major && differ);
            }
        }

        let yedge = if y == p.y_top {
            0x4 // edge flag: top edge
        } else if last_line {
            0x8 // edge flag: bottom edge
        } else {
            0
        };
        (
            Span {
                xstart,
                xend,
                l_edgelen,
                r_edgelen,
                l_edgecov,
                r_edgecov,
                l_filledge,
                r_filledge,
                zl,
                zr,
                interp_x: Interp::new(xstart, xend + 1, wl, wr, p.w_buffer),
                yedge,
                width: f.width,
            },
            swapped,
        )
    }

    /// Calls `f(x, edge_flags, segment)` for the pixels of the three span
    /// segments (0 = left edge, 1 = interior, 2 = right edge), honoring the
    /// fill rules.
    pub(super) fn for_span(span: &Span, wireframe: bool, mut f: impl FnMut(i32, u32, u32)) {
        let mut x = span.xstart.max(0);

        let mut xlimit = (span.xstart + span.l_edgelen).min(span.xend + 1).min(span.width);
        if span.l_filledge {
            while x < xlimit {
                f(x, span.yedge | 0x1, 0); // edge flag: left edge
                x += 1;
            }
        } else {
            x = x.max(xlimit);
        }

        xlimit = (span.xend - span.r_edgelen + 1).min(span.xend + 1).min(span.width);
        if wireframe && span.yedge == 0 {
            x = x.max(xlimit);
        } else {
            while x < xlimit {
                f(x, span.yedge, 1);
                x += 1;
            }
        }

        xlimit = (span.xend + 1).min(span.width);
        if span.r_filledge {
            while x < xlimit {
                f(x, span.yedge | 0x2, 2); // edge flag: right edge
                x += 1;
            }
        }
    }

    /// Shadow-mask polygon row: marks stencil bits where the depth test fails.
    pub(super) fn render_shadow_mask_scanline(
        &mut self,
        f: &Frame,
        rp: &mut RendererPolygon,
        y: i32,
    ) {
        let p = &f.polys[rp.poly];
        let depth_test = DepthTest::for_poly(p);
        let poly_alpha = (p.attr >> 16) & 0x1F; // POLYGON_ATTR alpha (bits 16-20)
        let wireframe = poly_alpha == 0;

        let sbase = self.width * (y as usize & 1);
        if !self.prev_is_shadow_mask {
            self.stencil[sbase..sbase + self.width].fill(0);
        }
        self.prev_is_shadow_mask = true;

        let layer = self.layer;
        let (span, _) = Self::setup_span(f, rp, y);
        let alpha = if wireframe { 31 } else { poly_alpha };
        if alpha > f.st.alpha_ref {
            let (zl, zr) = (span.zl, span.zr);
            let mut ix = span.interp_x;
            let row = self.row(y);
            let (depth, attr, stencil) = (&self.depth, &self.attr, &mut self.stencil);
            Self::for_span(&span, wireframe, |x, _, _| {
                let addr = row + x as usize;
                ix.set_x(x);
                let z = ix.interpolate_z(zl, zr);
                let dstattr = attr[addr];
                if !depth_test.test(depth[addr], z, dstattr) {
                    stencil[sbase + x as usize] = 1;
                }
                if dstattr & 0xF != 0 {
                    // edge flags (bits 0-3) set: pixel lies on a polygon edge
                    let a2 = addr + layer;
                    if !depth_test.test(depth[a2], z, attr[a2]) {
                        stencil[sbase + x as usize] |= 2;
                    }
                }
            });
        }

        rp.xl = rp.slope_l.step();
        rp.xr = rp.slope_r.step();
    }

    /// One polygon row: interpolate edges, then per pixel depth test,
    /// texturing/shading (render_pixel), alpha test, opaque write or
    /// plot_translucent_pixel, edge/AA coverage.
    #[expect(clippy::too_many_lines)]
    pub(super) fn render_polygon_scanline(&mut self, f: &Frame, rp: &mut RendererPolygon, y: i32) {
        let p = &f.polys[rp.poly];
        let mut polyattr = p.attr & 0x3F00_8000; // keep polygon ID (bits 24-29) and fog flag (bit 15)
        if !p.facing_view {
            polyattr |= 1 << 4; // internal flag: pixel belongs to a translucent polygon
        }
        let poly_alpha = (p.attr >> 16) & 0x1F; // POLYGON_ATTR alpha (bits 16-20)
        let wireframe = poly_alpha == 0;
        let depth_test = DepthTest::for_poly(p);
        self.prev_is_shadow_mask = false;

        let layer = self.layer;
        let (span, swapped) = Self::setup_span(f, rp, y);

        // Edge attribute interpolation.
        let (vlcur, vlnext, vrcur, vrnext, istart, iend) = if swapped {
            (
                f.vtx(p, rp.cur_vr),
                f.vtx(p, rp.next_vr),
                f.vtx(p, rp.cur_vl),
                f.vtx(p, rp.next_vl),
                rp.slope_r.interp,
                rp.slope_l.interp,
            )
        } else {
            (
                f.vtx(p, rp.cur_vl),
                f.vtx(p, rp.next_vl),
                f.vtx(p, rp.cur_vr),
                f.vtx(p, rp.next_vr),
                rp.slope_l.interp,
                rp.slope_r.interp,
            )
        };
        let cl: [i32; 3] = core::array::from_fn(|k| {
            istart.interpolate(vlcur.final_color[k], vlnext.final_color[k])
        });
        let tl: [i32; 2] = core::array::from_fn(|k| {
            istart.interpolate(i32::from(vlcur.texcoords[k]), i32::from(vlnext.texcoords[k]))
        });
        let cr: [i32; 3] =
            core::array::from_fn(|k| iend.interpolate(vrcur.final_color[k], vrnext.final_color[k]));
        let tr: [i32; 2] = core::array::from_fn(|k| {
            iend.interpolate(i32::from(vrcur.texcoords[k]), i32::from(vrnext.texcoords[k]))
        });

        let aa = f.st.disp_cnt & (1 << 4) != 0; // DISP3DCNT bit 4: anti-aliasing
        let alpha_ref = f.st.alpha_ref;
        let row = self.row(y);
        let sbase = self.width * (y as usize & 1);
        let (zl, zr) = (span.zl, span.zr);
        let mut ix = span.interp_x;

        let start_cov = |c: u32| {
            if c & (1 << 31) != 0 {
                // internal edge flag in bit 31
                let v = (c >> 12) & 0x3FF; // 10-bit vertex color component
                if v == 0x3FF { 0 } else { v } // saturated value reads as 0
            } else {
                0
            }
        };
        let mut xcov = start_cov(span.l_edgecov);
        let mut right_started = false;

        Self::for_span(&span, wireframe, |x, edge, seg| {
            if seg == 2 && !right_started {
                right_started = true;
                xcov = start_cov(span.r_edgecov);
            }
            let mut addr = row + x as usize;
            let mut dstattr = self.attr[addr];

            if p.is_shadow {
                let stencil = self.stencil[sbase + x as usize];
                if stencil == 0 {
                    return;
                }
                if stencil & 1 == 0 {
                    addr += layer;
                }
                if stencil & 2 == 0 {
                    dstattr &= !0xF; // clear edge flags (bits 0-3)
                }
            }

            ix.set_x(x);
            let mut z = ix.interpolate_z(zl, zr);

            if !depth_test.test(self.depth[addr], z, dstattr) {
                if dstattr & 0xF == 0 || addr >= layer {
                    // edge flags (bits 0-3) clear, or a deeper layer
                    return;
                }
                addr += layer;
                dstattr = self.attr[addr];
                if !depth_test.test(self.depth[addr], z, dstattr) {
                    return;
                }
            }

            let vr = ix.interpolate(cl[0], cr[0]) as u32;
            let vg = ix.interpolate(cl[1], cr[1]) as u32;
            let vb = ix.interpolate(cl[2], cr[2]) as u32;
            let s = ix.interpolate(tl[0], tr[0]) as i16;
            let t = ix.interpolate(tl[1], tr[1]) as i16;

            let color = f.render_pixel(
                p,
                (vr >> 3) & 0x3F, // 9-bit interpolated red to 6 bits
                (vg >> 3) & 0x3F, // 9-bit interpolated green to 6 bits
                (vb >> 3) & 0x3F, // 9-bit interpolated blue to 6 bits
                s,
                t,
            );
            let alpha = color >> 24;
            if alpha <= alpha_ref {
                return;
            }

            if alpha == 31 {
                let mut attr = polyattr | edge;
                // Interior pixels only take part in AA on the top/bottom line.
                if aa && (seg != 1 || attr & 0xF != 0) {
                    let cov = match seg {
                        0 => {
                            let c = span.l_edgecov;
                            if c & (1 << 31) != 0 {
                                // internal edge flag in bit 31
                                let v = (xcov >> 5).min(31);
                                xcov += c & 0x3FF; // 10-bit edge coverage fraction
                                v
                            } else {
                                c
                            }
                        }
                        2 => {
                            let c = span.r_edgecov;
                            if c & (1 << 31) != 0 {
                                // DS 3D Toon, Edge, Fog, Alpha-Blending, Anti-Aliasing: https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing
                                let v = 0x1F_u32.saturating_sub(xcov >> 5); // anti-aliasing coverage (0-31)
                                xcov += c & 0x3FF; // 10-bit edge coverage fraction
                                v
                            } else {
                                c
                            }
                        }
                        _ => 0x1F, // full coverage
                    };
                    attr |= cov << 8;
                    if addr < layer {
                        self.color[addr + layer] = self.color[addr];
                        self.depth[addr + layer] = self.depth[addr];
                        self.attr[addr + layer] = self.attr[addr];
                    }
                }
                self.depth[addr] = z;
                self.color[addr] = color;
                self.attr[addr] = attr;
            } else {
                if p.attr & (1 << 11) == 0 {
                    // POLYGON_ATTR bit 11: translucent pixels update the depth buffer
                    z = -1;
                }
                self.plot_translucent_pixel(f.st, addr, color, z, polyattr, p.is_shadow);
                if dstattr & 0xF != 0 && addr < layer {
                    // edge flags (bits 0-3) set on a deeper layer
                    self.plot_translucent_pixel(
                        f.st,
                        addr + layer,
                        color,
                        z,
                        polyattr,
                        p.is_shadow,
                    );
                }
            }
        });

        rp.xl = rp.slope_l.step();
        rp.xr = rp.slope_r.step();
    }

    /// Translucent pixel: polygon-ID rule, optional depth update, alpha blend.
    pub(super) fn plot_translucent_pixel(
        &mut self,
        st: &RenderState,
        addr: usize,
        color: u32,
        z: i32,
        polyattr: u32,
        shadow: bool,
    ) {
        let dstattr = self.attr[addr];
        let mut attr = (polyattr & 0xE0F0) // keep polygon fog/flag bits from POLYGON_ATTR
            | ((polyattr >> 8) & 0xFF_0000) // translucent polygon ID (POLYGON_ATTR bits 24-29) into bits 16-21
            | (1 << 22) // internal flag: translucent pixel already written
            | (dstattr & 0xFF00_1F0F); // keep destination edge flags, coverage and opaque polygon ID

        if shadow {
            if dstattr & (1 << 22) != 0 {
                // internal flag: translucent pixel already written
                if dstattr & 0x007F_0000 == attr & 0x007F_0000 {
                    // same translucent polygon ID already drawn here: skip
                    return;
                }
            } else if dstattr & 0x3F00_0000 == polyattr & 0x3F00_0000 {
                // same opaque polygon ID underneath
                return;
            }
        } else if dstattr & 0x007F_0000 == attr & 0x007F_0000 {
            // Same translucent polygon ID already drawn here.
            return;
        }

        if dstattr & (1 << 15) == 0 {
            // bit 15: fog enable of the destination
            attr &= !(1 << 15); // clear fog enable (bit 15)
        }

        let blended = Raster::alpha_blend(st, color, self.color[addr], color >> 24);
        if z != -1 {
            self.depth[addr] = z;
        }
        self.color[addr] = blended;
        self.attr[addr] = attr;
    }
}
