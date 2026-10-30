// SPDX-License-Identifier: GPL-3.0-or-later
//! High-resolution 3D for display only.
//!
//! The emulated hardware keeps using the native 256x192 raster (BG0,
//! display capture). When an internal resolution above 1x is chosen, the
//! inputs of every natively rendered frame are copied, and the frontend asks
//! for the scaled frame when it draws, so fast-forwarded frames that are
//! never shown cost only that copy. The copy ([`Snapshot`], with its
//! textures) is shared, so the scaled frame can also be rendered on another
//! thread with its own [`Renderer`] (see `display::hires::HdFrame`).
//!
//! ```text
//!  Gpu::render_3d_frame (line 215)
//!     native Raster::render ---> capture(): render state, polygons, vertices,
//!                                          textures (only when the native
//!                                          frame changed)
//!  frontend draws a frame
//!     Renderer::render(): vertices at hires_position * scale,
//!                          polygon bounds recomputed, same rasterizer at
//!                          scale x 256 x 192, textures from the native raster
//!     display::hires composites it over the upscaled 2D picture
//! ```
use std::sync::Arc;

use super::Raster;
use crate::engine_3d::state::{Polygon, RenderState, Vertex};

/// Inputs of one native frame.
#[derive(Debug)]
pub struct Snapshot {
    st: RenderState,
    polys: Vec<Polygon>,
    verts: Vec<Vertex>,
    order: Vec<usize>,
    /// Texture image and palette VRAM the native frame used.
    tex: Vec<u8>,
    pal: Vec<u8>,
    /// Native raster generation these inputs produced.
    generation: u64,
}

/// High-resolution 3D state (owned by `Gpu3D`).
#[derive(Debug)]
pub struct Hires {
    scale: usize,
    snapshot: Option<Arc<Snapshot>>,
    /// Renders for `Gpu::screen_hd` (other threads bring their own).
    pub(crate) renderer: Renderer,
}

impl Default for Hires {
    fn default() -> Self {
        Self { scale: 1, snapshot: None, renderer: Renderer::default() }
    }
}

impl Hires {
    /// The internal resolution factor (1 = off).
    pub const fn scale(&self) -> usize {
        self.scale
    }

    /// Sets the internal resolution factor (1 turns it off).
    pub fn set_scale(&mut self, scale: usize) {
        let scale = scale.clamp(1, 8);
        if scale == self.scale {
            return;
        }
        self.scale = scale;
        self.renderer = Renderer::default();
        if scale == 1 {
            self.snapshot = None;
        }
    }

    /// Copies the inputs of the frame the native raster holds, if they are
    /// new (or nothing was copied yet).
    pub(crate) fn capture(
        &mut self,
        native: &Raster,
        st: &RenderState,
        polys: &[Polygon],
        order: &[usize],
        verts: &[Vertex],
    ) {
        if self.scale == 1
            || self.snapshot.as_ref().is_some_and(|s| s.generation == native.generation())
        {
            return;
        }
        let poly_end = order.iter().max().map_or(0, |&i| i + 1).min(polys.len());
        let polys = &polys[..poly_end];
        let vert_end = order
            .iter()
            .filter_map(|&i| polys.get(i))
            .flat_map(|p| p.vertices[..p.num_vertices].iter())
            .max()
            .map_or(0, |&v| v + 1)
            .min(verts.len());
        self.snapshot = Some(Arc::new(Snapshot {
            st: st.clone(),
            polys: polys.to_vec(),
            verts: verts[..vert_end].to_vec(),
            order: order.to_vec(),
            tex: native.tex().to_vec(),
            pal: native.pal().to_vec(),
            generation: native.generation(),
        }));
    }

    /// The inputs of the last native frame (`None` at 1x or before the
    /// first 3D frame).
    pub fn snapshot(&self) -> Option<Arc<Snapshot>> {
        self.snapshot.clone()
    }
}

/// Renders [`Snapshot`]s at a scale, keeping the last result.
#[derive(Debug, Default)]
pub struct Renderer {
    raster: Option<Raster>,
    /// Snapshot generation the scaled raster holds.
    rendered: Option<u64>,
}

impl Renderer {
    /// `snap` rendered at `scale` (re-rendered only if it is a new frame).
    pub fn render(&mut self, snap: &Snapshot, scale: usize) -> &Raster {
        if self.raster.as_ref().is_some_and(|r| r.scale() != scale) {
            self.raster = None;
            self.rendered = None;
        }
        if self.rendered != Some(snap.generation) {
            let s = scale as i32;
            let verts: Vec<Vertex> = snap
                .verts
                .iter()
                .map(|v| Vertex {
                    final_position: [
                        (v.hires_position[0] * s) >> 4,
                        (v.hires_position[1] * s) >> 4,
                    ],
                    ..*v
                })
                .collect();
            let polys: Vec<Polygon> =
                snap.polys.iter().map(|p| scaled_bounds(p, &verts, s)).collect();
            let raster = self.raster.get_or_insert_with(|| Raster::with_scale(scale));
            raster.render_with(&snap.st, &polys, &snap.order, &verts, &snap.tex, &snap.pal);
            self.rendered = Some(snap.generation);
        }
        self.raster.get_or_insert_with(|| Raster::with_scale(scale))
    }
}

/// `p` with its top / bottom vertex and Y range taken from the scaled
/// vertices (same rules as polygon setup in the geometry engine).
fn scaled_bounds(p: &Polygon, verts: &[Vertex], s: i32) -> Polygon {
    let mut q = *p;
    let (mut ytop, mut ybot, mut xtop, mut xbot) = (192 * s, 0, 256 * s, 0);
    for i in 0..p.num_vertices {
        let Some(v) = verts.get(p.vertices[i]) else {
            q.degenerate = true;
            return q;
        };
        let [fx, fy] = v.final_position;
        if fy < ytop {
            xtop = fx;
            ytop = fy;
            q.v_top = i;
        }
        if fy > ybot || (fy == ybot && fx > xbot) {
            xbot = fx;
            ybot = fy;
            q.v_bottom = i;
        }
    }
    q.y_top = ytop;
    q.y_bottom = ybot;
    q.x_top = xtop;
    q.x_bottom = xbot;
    q
}
