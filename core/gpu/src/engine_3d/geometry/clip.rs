//! Clipping against the view volume (melonDS `ClipPolygon`), and the
//! BOX_TEST command that reuses it. See GBATEK "DS 3D Polygon Definitions"
//! and "DS 3D Tests".
//!
//! ```text
//!   view volume in clip space: -w <= x <= w, -w <= y <= w, -w <= z <= w
//!   (6 planes). A polygon partly outside is cut at each plane in turn:
//!
//!                    | plane x = w
//!        v0 *--------a----* v1           v0 *--------a
//!            \       |   /                   \       |
//!             \      |  /        ==>          \      |
//!              \     | /                       \     |
//!               \    |/                         \    |
//!                \   b                           \   b
//!                 \ /|                            \ /
//!                  * v2                            * v2
//!        v1 is outside:  v0 v1 v2  ->  v0 a b v2
//!
//!   new vertices (a, b) sit where an edge crosses the plane; their color and
//!   texture coordinates are interpolated along the edge. A clipped polygon
//!   can grow to 10 vertices. Polygons entirely outside are dropped.
//!   POLYGON_ATTR bit 12 = 0 drops polygons crossing the far plane instead.
//!
//!   BOX_TEST runs the same clipper on the 6 faces of a box and sets GXSTAT
//!   bit 1 if any part of it is inside (used to skip invisible objects).
//! ```
//!
//! GBATEK: [DS 3D Polygon Definitions by Vertices](https://problemkaputt.de/gbatek.htm#ds3dpolygondefinitionsbyvertices),
//! [DS 3D Tests](https://problemkaputt.de/gbatek.htm#ds3dtests)
use super::matrix::transform;
use crate::engine_3d::state::{Gpu3D, Vertex};

/// melonDS `ClipSegment`: places a vertex where the edge `vin`-`vout`
/// crosses the `plane * w` boundary of component `comp`.
pub(super) fn clip_segment(
    comp: usize,
    plane: i32,
    attribs: bool,
    vin: &Vertex,
    vout: &Vertex,
) -> Vertex {
    let factor_num = i64::from(vin.position[3]) - i64::from(plane) * i64::from(vin.position[comp]);
    let factor_den = (factor_num
        - (i64::from(vout.position[3]) - i64::from(plane) * i64::from(vout.position[comp])))
        as i32;
    let interp = |a: i32, b: i32| -> i32 {
        if factor_den == 0 {
            return a;
        }
        // 128-bit: the product exceeds 64 bits for extreme geometry.
        (i128::from(a)
            + (i128::from(b) - i128::from(a)) * i128::from(factor_num) / i128::from(factor_den))
            as i32
    };

    let mut out = *vin;
    for i in 0..4 {
        if i != comp {
            out.position[i] = interp(vin.position[i], vout.position[i]);
        }
    }
    out.position[comp] = plane.wrapping_mul(out.position[3]);
    if attribs {
        for i in 0..3 {
            out.color[i] = interp(vin.color[i], vout.color[i]);
        }
        for i in 0..2 {
            out.texcoords[i] =
                interp(i32::from(vin.texcoords[i]), i32::from(vout.texcoords[i])) as i16;
        }
    }
    out.clipped = true;
    out
}

/// melonDS `ClipAgainstPlane`.
pub(super) fn clip_against_plane(
    comp: usize,
    attribs: bool,
    far_intersect: bool,
    vertices: &mut [Vertex; 10],
    nverts: usize,
    clip_start: usize,
) -> usize {
    let mut temp = [Vertex::default(); 10];
    let mut c = clip_start;
    if clip_start == 2 {
        temp[0] = vertices[0];
        temp[1] = vertices[1];
    }

    for i in clip_start..nverts {
        let prev = if i == 0 { nverts - 1 } else { i - 1 };
        let next = if i + 1 >= nverts { 0 } else { i + 1 };
        let vtx = vertices[i];
        if vtx.position[comp] > vtx.position[3] {
            if comp == 2 && !far_intersect {
                return 0;
            }
            let vprev = vertices[prev];
            if vprev.position[comp] <= vprev.position[3] && c < 10 {
                temp[c] = clip_segment(comp, 1, attribs, &vtx, &vprev);
                c += 1;
            }
            let vnext = vertices[next];
            if vnext.position[comp] <= vnext.position[3] && c < 10 {
                temp[c] = clip_segment(comp, 1, attribs, &vtx, &vnext);
                c += 1;
            }
        } else if c < 10 {
            temp[c] = vtx;
            c += 1;
        }
    }

    let nverts = c;
    c = clip_start;
    for i in clip_start..nverts {
        let prev = if i == 0 { nverts - 1 } else { i - 1 };
        let next = if i + 1 >= nverts { 0 } else { i + 1 };
        let vtx = temp[i];
        if i64::from(vtx.position[comp]) < -i64::from(vtx.position[3]) {
            let vprev = temp[prev];
            if i64::from(vprev.position[comp]) >= -i64::from(vprev.position[3]) && c < 10 {
                vertices[c] = clip_segment(comp, -1, attribs, &vtx, &vprev);
                c += 1;
            }
            let vnext = temp[next];
            if i64::from(vnext.position[comp]) >= -i64::from(vnext.position[3]) && c < 10 {
                vertices[c] = clip_segment(comp, -1, attribs, &vtx, &vnext);
                c += 1;
            }
        } else if c < 10 {
            vertices[c] = vtx;
            c += 1;
        }
    }

    for v in &mut vertices[..c] {
        for col in &mut v.color {
            *col = (*col & !0xFFF) + 0xFFF;
        }
    }
    c
}

/// Clips against the Z, Y and X planes (melonDS `ClipPolygon`).
pub(super) fn clip_polygon(
    attribs: bool,
    far_intersect: bool,
    vertices: &mut [Vertex; 10],
    nverts: usize,
    clip_start: usize,
) -> usize {
    let n = clip_against_plane(2, attribs, far_intersect, vertices, nverts, clip_start);
    let n = clip_against_plane(1, attribs, far_intersect, vertices, n, clip_start);
    clip_against_plane(0, attribs, far_intersect, vertices, n, clip_start)
}

impl Gpu3D {
    /// melonDS `BoxTest`: whether any face of the box is inside the view
    /// volume.
    pub(super) fn box_test(&mut self, p: &[u32; 32]) {
        // DS 3D Tests: https://problemkaputt.de/gbatek.htm#ds3dtests
        self.gxstat &= !(1 << 1); // GXSTAT bit 1: box test result (clear)
        let x0 = p[0] as i16;
        let y0 = (p[0] >> 16) as i16;
        let z0 = p[1] as i16;
        let x1 = ((p[1] >> 16) as i16).wrapping_add(x0);
        let y1 = (p[2] as i16).wrapping_add(y0);
        let z1 = ((p[2] >> 16) as i16).wrapping_add(z0);
        let corners = [
            [x0, y0, z0],
            [x1, y0, z0],
            [x1, y1, z0],
            [x0, y1, z0],
            [x0, y1, z1],
            [x0, y0, z1],
            [x1, y0, z1],
            [x1, y1, z1],
        ];
        self.update_clip_matrix();
        let cube: [Vertex; 8] = corners.map(|c| Vertex {
            position: transform(c.map(i32::from), &self.clip_matrix),
            ..Vertex::default()
        });
        const FACES: [[usize; 4]; 6] =
            [[0, 1, 2, 3], [4, 5, 6, 7], [0, 3, 4, 5], [1, 2, 7, 6], [0, 1, 6, 5], [2, 3, 4, 7]];
        let far = self.cur_polygon_attr & (1 << 12) != 0; // POLYGON_ATTR bit 12: far plane intersecting polygons
        for f in FACES {
            let mut face = [Vertex::default(); 10];
            for (k, &i) in f.iter().enumerate() {
                face[k] = cube[i];
            }
            if clip_polygon(false, far, &mut face, 4, 0) > 0 {
                self.gxstat |= 1 << 1; // GXSTAT bit 1: box test result = inside
                return;
            }
        }
    }
}
