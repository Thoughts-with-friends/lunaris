// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! DS 3D engine: geometry engine ([`geometry`]), registers and buffer swap
//! ([`regs`]) and the software rasterizer ([`raster`]).
//!
//! ## 3D pipeline at a glance
//!
//! ```text
//!   CPU writes commands                geometry engine (geometry/)
//!   GXFIFO 04000400h or     ------->   matrices: projection, position, vector, texture
//!   command ports 04000440h+           every VTX_*: vertex * clip matrix
//!   (MTX_*, COLOR, TEXCOORD,           every 3 / 4 vertices -> one polygon:
//!    NORMAL, VTX_*, BEGIN_VTXS ...)       cull -> clip -> viewport -> polygon RAM
//!                                                      |
//!                         SWAP_BUFFERS, takes effect at VBlank (double buffer:
//!                         the game builds frame n+1 while frame n is drawn)
//!                                                      v
//!                                      rendering engine (raster/)
//!                                        clear, fill polygons scanline by
//!                                        scanline: texture, shading, depth test,
//!                                        alpha blend; then edge marking, fog,
//!                                        anti-aliasing
//!                                                      |
//!                                                      v
//!                                      256x192 image -> engine A BG0
//!
//!   coordinate spaces of one vertex:
//!     model (x, y, z, 1)
//!       --position matrix-->   view space (camera at the origin)
//!       --projection matrix--> clip space (x, y, z, w)
//!                              visible when -w <= x, y, z <= w
//!       --divide by w, VIEWPORT-->  screen:
//!              screenX = (x/w + 1) / 2 * width  + X1
//!              screenY = (1 - y/w) / 2 * height + top     (Y grows downward)
//!       depth: z/w (Z-buffering) or w (W-buffering, SWAP_BUFFERS bit 1)
//!
//!     clip space                 screen (VIEWPORT 0,0 - 255,191)
//!        y=+w                    (0,0) +-----------------+
//!     +---------+                      |                 |
//!     |    .    |  x=+w    ---->       |        .        |
//!     |  (0,0)  |                      |    (128,96)     |
//!     +---------+                      +-----------------+ (255,191)
//!        y=-w
//! ```
//!
//! GBATEK: [DS 3D Video](https://problemkaputt.de/gbatek.htm#ds3dvideo),
//! [DS 3D Overview](https://problemkaputt.de/gbatek.htm#ds3doverview)
pub mod geometry;
pub mod gxfifo;
pub mod raster;
pub mod regs;
pub mod state;

use crate::Gpu;

impl Gpu {
    /// Renders the 3D frame swapped in at the last VBlank. It is shown on
    /// engine A's BG0 during the next frame.
    ///
    /// ```text
    /// from: ds_emu gpu_handle_event() at line 215 (after the VBlank
    ///       texture uploads)
    ///   |
    ///   v
    /// render_3d_frame()
    ///   1. 3D not displayed (DISPCNT bit 3)?  mark dirty, skip
    ///   2. raster.render(latched registers, polygon/vertex RAM of the
    ///      render bank, sorted render list, VRAM textures, dirty flag)
    ///   |
    ///   v
    /// to:   compose_scanline() -> render_bg_3d() reads raster.line(y)
    /// ```
    pub fn render_3d_frame(&mut self) {
        let e = &mut self.engine_3d;
        if !self.engine_upper.dispcnt.bg_3d {
            // Not displayed: skip, and render once 3D is shown again.
            e.render_dirty = true;
            return;
        }
        let bank = e.render_bank;
        e.raster.render(
            &e.render,
            &e.polygon_ram[bank],
            &e.render_polygons,
            &e.vertex_ram[bank],
            &self.vram,
            e.render_dirty,
        );
        e.render_dirty = false;
        e.hires.capture(
            &e.raster,
            &e.render,
            &e.polygon_ram[bank],
            &e.render_polygons,
            &e.vertex_ram[bank],
        );
    }

    /// Sets the internal resolution of the display-only 3D copy (1 = off,
    /// see [`raster::hires`]).
    pub fn set_render_scale(&mut self, scale: usize) {
        self.engine_3d.hires.set_scale(scale);
    }

    /// The 3D frame at the internal resolution, if one is set and 3D was
    /// rendered (renders it now if the native frame changed).
    pub fn hires_3d(&mut self) -> Option<&raster::Raster> {
        let hires = &mut self.engine_3d.hires;
        let snap = hires.snapshot()?;
        let scale = hires.scale();
        Some(hires.renderer.render(&snap, scale))
    }
}
