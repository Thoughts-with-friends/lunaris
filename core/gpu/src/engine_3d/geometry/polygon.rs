//! Vertex submission and polygon setup: strips, culling, clipping, viewport
//! transform, W normalization and depth (melonDS `SubmitVertex` /
//! `SubmitPolygon`). See GBATEK "DS 3D Polygon Definitions".
//!
//! ## From vertices to polygons
//!
//! ```text
//!   BEGIN_VTXS type (vertex order as sent):
//!
//!    0 triangles      1 quads             2 triangle strip     3 quad strip
//!    0     3          0---3   4---7       0---2---4            0---2---4
//!    |\    |\         |   |   |   |       |\  |\  |            |   |   |
//!    | \   | \        |   |   |   |       | \ | \ |            |   |   |
//!    1--2  4--5       1---2   5---6       1---3---5            1---3---5
//!                                         (0,1,2) (1,3,2)...   (0,1,3,2) (2,3,5,4)
//!
//!   Strips reuse the last 2 vertices, so each extra vertex (or pair) adds
//!   a polygon.
//!
//!   culling: the winding of the projected vertices tells which side faces the
//!   viewer. POLYGON_ATTR bit 7 = draw front faces, bit 6 = draw back faces.
//!
//!      anti-clockwise on screen   clockwise on screen
//!           0                          0
//!          / \                        / \
//!         1---2   = front            2---1   = back
//!      (GBATEK: vertices are normally sent anti-clockwise)
//!
//!   after culling: clip (clip.rs) -> divide by w -> viewport -> depth
//!   -> stored in polygon RAM (max 2048 polygons, 6144 vertices; overflow sets
//!      DISP3DCNT bit 13)
//!
//!   render order at the buffer swap: opaque polygons first, sorted by their
//!   bottom/top Y, then translucent ones (Y-sorted too unless SWAP_BUFFERS
//!   bit 0 asks for manual order).
//! ```
//!
//! GBATEK: [DS 3D Polygon Definitions by Vertices](https://problemkaputt.de/gbatek.htm#ds3dpolygondefinitionsbyvertices),
//! [DS 3D Polygon Attributes](https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes)
use super::{clip::clip_polygon, matrix::transform, mul};
use crate::engine_3d::state::{Gpu3D, MAX_POLYGONS, MAX_VERTICES, Polygon, Vertex};

impl Gpu3D {
    /// melonDS `SubmitVertex`.
    ///
    /// ```text
    /// from: execute_command() VTX_* commands
    ///   1. transform by the clip matrix (projection * position)
    ///   2. store with the current color / texcoord in the vertex slot
    ///   3. enough vertices for the primitive (tri, quad, strips)?
    ///        -> submit_polygon()
    /// ```
    pub(super) fn submit_vertex(&mut self) {
        self.update_clip_matrix();
        let v = self.cur_vertex.map(i32::from);
        let idx = self.vertex_num_in_poly;
        let mut vt = Vertex {
            position: transform(v, &self.clip_matrix),
            // DS 3D Polygon Attributes: https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes
            color: self.vertex_color.map(|c| (i32::from(c) << 12) + 0xFFF), // 5-bit color to fixed point, low bits filled
            ..Vertex::default()
        };
        if self.tex_param >> 30 == 3 {
            let m = &self.tex_matrix;
            for i in 0..2 {
                let t = (mul(v[0], m[i]) + mul(v[1], m[4 + i]) + mul(v[2], m[8 + i])) >> 24;
                vt.texcoords[i] = (t as i16).wrapping_add(self.raw_texcoords[i]);
            }
        } else {
            vt.texcoords = self.texcoords;
        }
        self.temp_vertices[idx] = vt;
        self.vertex_num_in_poly += 1;

        match self.polygon_mode {
            0 => {
                if self.vertex_num_in_poly == 3 {
                    self.vertex_num_in_poly = 0;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                }
            }
            1 => {
                if self.vertex_num_in_poly == 4 {
                    self.vertex_num_in_poly = 0;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                }
            }
            2 => {
                if self.num_consecutive_polygons & 1 != 0 {
                    self.temp_vertices.swap(0, 1);
                    self.vertex_num_in_poly = 2;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                    self.temp_vertices[1] = self.temp_vertices[2];
                } else if self.vertex_num_in_poly == 3 {
                    self.vertex_num_in_poly = 2;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                    self.temp_vertices[0] = self.temp_vertices[1];
                    self.temp_vertices[1] = self.temp_vertices[2];
                }
            }
            _ => {
                if self.vertex_num_in_poly == 4 {
                    self.temp_vertices.swap(2, 3);
                    self.vertex_num_in_poly = 2;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                    self.temp_vertices[0] = self.temp_vertices[3];
                    self.temp_vertices[1] = self.temp_vertices[2];
                }
            }
        }
    }

    /// melonDS `SubmitPolygon`: culling, clipping, viewport transform and
    /// polygon setup.
    ///
    /// ```text
    /// from: submit_vertex()
    ///   1. back/front-face culling per POLYGON_ATTR
    ///   2. clip_polygon() against the view volume (clip.rs)
    ///   3. viewport transform to screen x/y, depth (Z or W buffering)
    ///   4. append to polygon RAM (+ vertex RAM); count opaque ones and
    ///      compute the Y sort key
    /// to:   end_of_frame() turns the list into the render list
    /// ```
    #[expect(clippy::too_many_lines)]
    pub(super) fn submit_polygon(&mut self) {
        let mut nverts = if self.polygon_mode & 1 != 0 { 4 } else { 3 };
        let attr = self.cur_polygon_attr;

        // Culling (facing test in homogeneous coordinates).
        let [v0, v1, v2, _] = self.temp_vertices;
        // Differences wrap in 32 bits, products are 64-bit (as in melonDS).
        let d =
            |a: &Vertex, b: &Vertex, i: usize| i64::from(a.position[i].wrapping_sub(b.position[i]));
        let cross = |a0: i64, b0: i64, a1: i64, b1: i64| (a0 * b0).wrapping_sub(a1 * b1);
        let mut nx = cross(d(&v0, &v1, 1), d(&v2, &v1, 3), d(&v0, &v1, 3), d(&v2, &v1, 1));
        let mut ny = cross(d(&v0, &v1, 3), d(&v2, &v1, 0), d(&v0, &v1, 0), d(&v2, &v1, 3));
        let mut nz = cross(d(&v0, &v1, 0), d(&v2, &v1, 1), d(&v0, &v1, 1), d(&v2, &v1, 0));
        let big = |n: i64| (n >> 31) ^ (n >> 63) != 0;
        while big(nx) || big(ny) || big(nz) {
            nx >>= 4;
            ny >>= 4;
            nz >>= 4;
        }
        let dot = i64::from(v1.position[0])
            .wrapping_mul(nx)
            .wrapping_add(i64::from(v1.position[1]).wrapping_mul(ny))
            .wrapping_add(i64::from(v1.position[3]).wrapping_mul(nz));
        let facing_view = dot <= 0;
        // DS 3D Polygon Attributes: https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes
        if (dot < 0 && attr & (1 << 7) == 0) || (dot > 0 && attr & (1 << 6) == 0) {
            // POLYGON_ATTR bit 7: render front surface, bit 6: render back surface
            self.last_strip_polygon = None;
            return;
        }

        // Strips: reuse the two shared vertices of the previous polygon.
        let bank = self.cur_bank;
        let mut clipped = [Vertex::default(); 10];
        let mut clip_start = 0;
        let mut reused = [0usize; 2];
        let mut last_poly_verts = 0;
        if self.polygon_mode >= 2
            && let Some(last) = self.last_strip_polygon
        {
            let (id0, id1) = if self.polygon_mode == 2 {
                last_poly_verts = 3;
                if self.num_consecutive_polygons & 1 != 0 { (2, 1) } else { (0, 2) }
            } else {
                last_poly_verts = 4;
                (3, 2)
            };
            let lp = &self.polygon_ram[bank][last];
            let vr = &self.vertex_ram[bank];
            if lp.num_vertices == last_poly_verts
                && !vr[lp.vertices[id0]].clipped
                && !vr[lp.vertices[id1]].clipped
            {
                reused = [lp.vertices[id0], lp.vertices[id1]];
                clipped[0] = vr[reused[0]];
                clipped[1] = vr[reused[1]];
                clip_start = 2;
            }
        }
        clipped[clip_start..nverts].copy_from_slice(&self.temp_vertices[clip_start..nverts]);

        nverts = clip_polygon(
            true,
            attr & (1 << 12) != 0, // POLYGON_ATTR bit 12: far plane intersecting polygons
            &mut clipped,
            nverts,
            clip_start,
        );
        if nverts == 0 {
            self.last_strip_polygon = None;
            return;
        }
        if self.num_polygons >= MAX_POLYGONS || self.num_vertices + nverts > MAX_VERTICES {
            self.last_strip_polygon = None;
            // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
            self.disp_cnt |= 1 << 13; // DISP3DCNT bit 13: polygon/vertex RAM overflow
            return;
        }

        // Viewport transform (32-bit divider, as on hardware).
        let vp = self.viewport;
        for vtx in &mut clipped[clip_start..nverts] {
            // DS 3D Polygon Definitions by Vertices: https://problemkaputt.de/gbatek.htm#ds3dpolygondefinitionsbyvertices
            vtx.position[3] &= 0x00FF_FFFF; // W is kept to 24 bits
            let w = vtx.position[3] as u32;
            let (pos_x, pos_y, fine) = if w == 0 {
                (0, 0, [0, 0])
            } else {
                let mut px = (vtx.position[0] as u32).wrapping_add(w);
                let mut py = (vtx.position[1] as u32).wrapping_neg().wrapping_add(w);
                let mut den = w;
                if w > 0xFFFF {
                    // W wider than 16 bits: normalize before dividing
                    px >>= 1;
                    py >>= 1;
                    den >>= 1;
                }
                den <<= 1;
                // The same position with 4 fractional bits (for rendering
                // above the native resolution).
                let fine = |p: u32, size: i32, base: i32| {
                    ((i64::from(p) * i64::from(size) * 16) / i64::from(den) + i64::from(base) * 16)
                        as i32
                };
                (
                    (px.wrapping_mul(vp.width as u32) / den).wrapping_add(vp.x0 as u32),
                    (py.wrapping_mul(vp.height as u32) / den).wrapping_add(vp.y1 as u32),
                    [fine(px, vp.width, vp.x0), fine(py, vp.height, vp.y1)],
                )
            };
            vtx.final_position = [(pos_x & 0x1FF) as i32, (pos_y & 0xFF) as i32]; // screen X is 9 bits, Y is 8 bits
            vtx.hires_position = [fine[0] & 0x1FFF, fine[1] & 0xFFF]; // hi-res subpixel coordinates (13 / 12 bits)
        }

        // Zero-dot polygons far behind are rejected unless bit 13 is set.
        if attr & (1 << 13) == 0 {
            let first = clipped[0].final_position;
            let mut zero_dot = true;
            let mut all_behind = true;
            for v in &clipped[..nverts] {
                if v.final_position != first {
                    zero_dot = false;
                    break;
                }
                if v.position[3] <= self.zero_dot_w_limit {
                    all_behind = false;
                    break;
                }
            }
            if zero_dot && all_behind {
                self.last_strip_polygon = None;
                return;
            }
        }

        let tex_fmt = (self.tex_param >> 26) & 7;
        // DS 3D Polygon Attributes: https://problemkaputt.de/gbatek.htm#ds3dpolygonattributes
        let alpha = (attr >> 16) & 0x1F; // POLYGON_ATTR alpha (bits 16-20)
        let mut poly = Polygon {
            attr,
            tex_param: self.tex_param,
            tex_palette: self.tex_palette,
            facing_view,
            translucent: tex_fmt == 1 || tex_fmt == 6 || (alpha > 0 && alpha < 31),
            is_shadow_mask: attr & 0x3F00_0030 == 0x30, // shadow mask polygon: mode 3 (bits 4-5) with polygon ID 0 (bits 24-29)
            ..Polygon::default()
        };
        poly.is_shadow = attr & 0x30 == 0x30 && !poly.is_shadow_mask; // shadow polygon: mode 3 (bits 4-5) with a non-zero ID

        let vram = &mut self.vertex_ram[bank];
        if self.last_strip_polygon.is_some() && clip_start > 0 {
            if nverts == last_poly_verts {
                poly.vertices[0] = reused[0];
                poly.vertices[1] = reused[1];
            } else {
                for k in 0..2 {
                    vram[self.num_vertices] = vram[reused[k]];
                    poly.vertices[k] = self.num_vertices;
                    self.num_vertices += 1;
                }
            }
            poly.num_vertices = 2;
        }
        for (i, src) in clipped.iter().enumerate().take(nverts).skip(clip_start) {
            let mut vtx = *src;
            for k in 0..3 {
                let c = vtx.color[k] >> 12;
                vtx.final_color[k] = if c != 0 { (c << 4) + 0xF } else { 0 }; // 5-bit color component expanded to 9 bits
            }
            vram[self.num_vertices] = vtx;
            poly.vertices[i] = self.num_vertices;
            self.num_vertices += 1;
            poly.num_vertices += 1;
        }

        // Bounds, W normalization and depth values.
        let (mut ytop, mut ybot, mut xtop, mut xbot) = (192, 0, 256, 0);
        let mut wsize = 0u32;
        for i in 0..nverts {
            let v = &vram[poly.vertices[i]];
            let [fx, fy] = v.final_position;
            if fy < ytop {
                xtop = fx;
                ytop = fy;
                poly.v_top = i;
            }
            if fy > ybot || (fy == ybot && fx > xbot) {
                xbot = fx;
                ybot = fy;
                poly.v_bottom = i;
            }
            let w = v.position[3] as u32;
            if w == 0 {
                poly.degenerate = true;
            }
            while wsize < 32 && (w >> wsize) != 0 {
                wsize += 4;
            }
        }
        poly.y_top = ytop;
        poly.y_bottom = ybot;
        poly.x_top = xtop;
        poly.x_bottom = xbot;
        if ybot > 192 {
            poly.degenerate = true;
        }
        poly.sort_key = ((ybot as u32) << 8) | ytop as u32;
        if poly.translucent {
            // DS 3D Toon, Edge, Fog, Alpha-Blending, Anti-Aliasing: https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing
            poly.sort_key |= 0x10000; // translucent polygons sort after opaque ones
        }
        poly.w_buffer = self.flush_attributes & 2 != 0;

        for i in 0..nverts {
            let v = &vram[poly.vertices[i]];
            let (w, wshifted) = if wsize < 16 {
                let w = v.position[3] << (16 - wsize);
                (w, w >> (16 - wsize))
            } else {
                let w = v.position[3] >> (wsize - 16);
                (w, w << (wsize - 16))
            };
            let z = if poly.w_buffer {
                i64::from(wshifted)
            } else if v.position[3] != 0 {
                // DS 3D Matrix Types: https://problemkaputt.de/gbatek.htm#ds3dmatrixtypes
                (i64::from(v.position[2]) * 0x4000 / i64::from(v.position[3]) + 0x3FFF) * 0x200 // Z-buffer depth: z / w scaled to 24 bits
            } else {
                0x7F_FE00 // W-buffering depth limit
            };
            poly.final_z[i] = z.clamp(0, 0xFF_FFFF) as i32; // depth buffer is 24 bits
            poly.final_w[i] = w;
        }

        if !poly.translucent {
            self.num_opaque_polygons += 1;
        }
        let index = self.num_polygons;
        self.polygon_ram[bank][index] = poly;
        self.num_polygons += 1;
        self.last_strip_polygon = (self.polygon_mode >= 2).then_some(index);
    }
}
