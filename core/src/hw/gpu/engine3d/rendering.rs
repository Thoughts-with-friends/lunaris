//! Software rasterizer emulating the NDS 3D rendering engine.
//!
//! GBATEK references:
//! - Rendering engine overview: <https://problemkaputt.de/gbatek.htm#ds3doverview>
//! - Texture blending (modulation/decal/toon):
//!   <https://problemkaputt.de/gbatek.htm#ds3dtextureblending>
//! - Texture formats: <https://problemkaputt.de/gbatek.htm#ds3dtextureformats>
//! - Final 2D output of 3D layer: <https://problemkaputt.de/gbatek.htm#ds3dfinal2doutput>

use super::{
    super::VRAM,
    Color, Engine3D, GPU, TextureFormat,
    geometry::{Polygon, Vertex},
    registers::{DISP3DCNT, PolygonMode},
};

impl Engine3D {
    pub fn pixel_color(&self, index: usize) -> u16 {
        self.frame_buffer[index].color.as_u16()
    }

    pub fn copy_line(&self, vcount: u16, line: &mut [u16; GPU::WIDTH]) {
        for (i, pixel) in line.iter_mut().enumerate() {
            *pixel = self.frame_buffer[vcount as usize * GPU::WIDTH + i].color.as_u16()
        }
    }

    /// Resolves the pending SwapBuffers and, when `rendering_enabled`,
    /// rasterizes all submitted polygons into the internal frame buffer.
    ///
    /// Runs once per frame after SwapBuffers; on hardware the rendering
    /// engine draws scanline-by-scanline starting 48 lines ahead of the
    /// display.  Clears to CLEAR_COLOR/CLEAR_DEPTH, then scan-converts each
    /// polygon with perspective-correct texturing, depth test, and toon /
    /// modulation blending.
    ///
    /// `rendering_enabled` mirrors POWCNT1 "Enable 3D Rendering" (bit 2).
    /// Per GBATEK "DS Power Control" (`#dspowercontrol`), that bit only
    /// gates the rasterizer - the geometry engine's SwapBuffers halt
    /// (`polygons_submitted`) must still be resolved every V-Blank
    /// regardless of it, otherwise the GXFIFO fills and the CPU bus stalls
    /// permanently. See `docs/design/3d-rendering-bugfix-design.md` §3.1.
    ///
    /// GBATEK "DS 3D Overview – Rendering Engine":
    /// <https://problemkaputt.de/gbatek.htm#ds3doverview>
    pub fn render(&mut self, vram: &VRAM, rendering_enabled: bool) {
        if !self.polygons_submitted {
            return;
        }

        // TEMP DIAGNOSTIC (docs/design/bw2-3d-building-render-design.md §4):
        // env-gated so normal runs are unaffected; removed once the
        // discriminator verdict is reached.
        if std::env::var_os("LUNARIS_3D_DEBUG").is_some() {
            eprintln!(
                "[3d-dbg] render() polygons_submitted={} polygons={} vertices={} clear_color=({},{},{},a={})",
                self.polygons_submitted,
                self.polygons.len(),
                self.vertices.len(),
                self.clear_color.r,
                self.clear_color.g,
                self.clear_color.b,
                self.clear_color.a,
            );
        }

        // GBATEK "DS 3D Polygon List Commands - SwapBuffers"
        // (`#ds3dpolygonlistcommands`): parameters passed to SwapBuffers
        // apply to the polygons defined after it, i.e. they take effect
        // for the frame being swapped in now.
        self.frame_params = self.next_frame_params;

        if !rendering_enabled {
            self.polygons.clear();
            self.vertices.clear();
            self.gxstat.geometry_engine_busy = false;
            self.polygons_submitted = false;
            return;
        }

        // TODO: Optimize
        for pixel in self.frame_buffer.iter_mut() {
            pixel.color = FrameBufferColor::new5(
                Color::new5(self.clear_color.r, self.clear_color.g, self.clear_color.b),
                self.clear_color.a,
            );
            pixel.depth = self.clear_depth.depth();
        }

        let w_buffer = self.frame_params.w_buffer;
        warn!("disp3dcnt.alpha_test is not implemented, so alpha test is currently disabled");
        // assert!(!self.disp3dcnt.alpha_test); // TODO: Implement alpha test

        let disp3dcnt = &self.disp3dcnt;
        let toon_table = &self.toon_table;
        let blend = |polygon: &Polygon, vert_color, s: i32, t: i32| {
            let tex_color = Self::get_tex_color(vram, polygon, s, t);
            let modulation_blend = |val1, val2| ((val1 + 1) * (val2 + 1) - 1) / 64;
            match polygon.attrs.mode {
                PolygonMode::Modulation => {
                    Self::blend_tex(tex_color, vert_color, modulation_blend, modulation_blend)
                }
                PolygonMode::ToonHighlight if disp3dcnt.highlight_shading => Self::blend_tex(
                    tex_color,
                    vert_color,
                    |val1, val2| std::cmp::max(modulation_blend(val1, val2) + val2, 0x3F),
                    modulation_blend,
                ),
                PolygonMode::ToonHighlight => {
                    let toon_color =
                        FrameBufferColor::new8(toon_table[vert_color.r5() as usize], vert_color.a);
                    Self::blend_tex(tex_color, toon_color, modulation_blend, modulation_blend)
                }
                // GBATEK "DS 3D Texture Blending" (`#ds3dtextureblending`),
                // Decal mode: texel RGB replaces the vertex RGB weighted by
                // the texel's own alpha (R = (Rt*At + Rv*(63-At))/64); the
                // vertex alpha always passes through untouched. Falls back
                // to the vertex color when there is no texture (mirrors
                // the other modes). This can't reuse `blend_tex` since its
                // per-component closures don't see the texel's alpha.
                PolygonMode::Decal => match tex_color {
                    Some(tex) => {
                        let tex_alpha = tex.a6() as u16;
                        let calc = |t: u16, v: u16| (t * tex_alpha + v * (63 - tex_alpha)) / 64;
                        FrameBufferColor::new6(
                            Color::new6(
                                calc(tex.r6() as u16, vert_color.r6() as u16) as u8,
                                calc(tex.g6() as u16, vert_color.g6() as u16) as u8,
                                calc(tex.b6() as u16, vert_color.b6() as u16) as u8,
                            ),
                            vert_color.a6(),
                        )
                    }
                    None => vert_color,
                },
                PolygonMode::Shadow => {
                    tex_color.unwrap_or_else(|| FrameBufferColor::new5(Color::new5(0, 0, 0), 0))
                }
            }
        };

        let vertices = &self.vertices;
        let frame_buffer = &mut self.frame_buffer;
        let mut render = |polygon: Polygon| {
            let vertices = &vertices[polygon.start_vert..polygon.end_vert];
            Self::render_polygon(disp3dcnt, w_buffer, blend, &polygon, vertices, frame_buffer);
        };

        if disp3dcnt.alpha_blending {
            let (opaque, mut translucent): (Vec<Polygon>, Vec<Polygon>) =
                self.polygons.drain(..).partition(|polygon| polygon.attrs.alpha == 0x1F);

            for polygon in opaque {
                render(polygon)
            }
            // GBATEK "DS 3D Rendering Engine": unless manual translucent
            // sorting is requested via SwapBuffers, translucent polygons
            // are Y-sorted back-to-front before compositing so overlapping
            // transparent surfaces blend in the correct order.
            if !self.frame_params.manual_sort_translucent {
                translucent.sort_by_key(|polygon| polygon.y_bounds.0);
            }
            for polygon in translucent {
                render(polygon)
            }
        } else {
            for polygon in self.polygons.drain(..) {
                render(polygon)
            }
        }

        self.vertices.clear();
        self.gxstat.geometry_engine_busy = false;
        self.polygons_submitted = false;
    }

    fn render_polygon<B>(
        disp3dcnt: &DISP3DCNT,
        w_buffer: bool,
        blend: B,
        polygon: &Polygon,
        vertices: &[Vertex],
        frame_buffer: &mut [FrameBufferPixel],
    ) where
        B: Fn(&Polygon, FrameBufferColor, i32, i32) -> FrameBufferColor,
    {
        if polygon.attrs.mode == PolygonMode::Shadow {
            return;
        }
        let depth_test = Self::get_depth_test(polygon);
        // Find top left and bottom right vertices
        let (mut start_vert, mut end_vert) = (0, 0);
        for (i, vert) in vertices.iter().enumerate() {
            #[expect(clippy::if_same_then_else)]
            if vert.screen_coords[1] < vertices[start_vert].screen_coords[1] {
                start_vert = i;
            } else if vert.screen_coords[1] == vertices[start_vert].screen_coords[1]
                && vert.screen_coords[0] < vertices[start_vert].screen_coords[0]
            {
                start_vert = i;
            }

            #[expect(clippy::if_same_then_else)]
            if vert.screen_coords[1] > vertices[end_vert].screen_coords[1] {
                end_vert = i;
            } else if vert.screen_coords[1] == vertices[end_vert].screen_coords[1]
                && vert.screen_coords[0] > vertices[end_vert].screen_coords[0]
            {
                end_vert = i;
            }
        }
        let mut left_vert = start_vert;
        let mut right_vert = start_vert;
        let start_vert = start_vert; // Shadow to mark these as immutable
        let end_vert = end_vert; // Shadow to mark these as immutable

        let next = |cur| {
            if cur == vertices.len() - 1 { 0 } else { cur + 1 }
        };
        let prev = |cur| {
            if cur == 0 { vertices.len() - 1 } else { cur - 1 }
        };
        // Winding direction picks which walk direction is the "left" edge;
        // plain function pointers avoid a per-polygon heap allocation that
        // a `Box<dyn Fn>` pair would otherwise incur (see
        // `docs/design/3d-rendering-bugfix-design.md` §5.2).
        let is_front = polygon.is_front;
        let next_left = |cur| if is_front { next(cur) } else { prev(cur) };
        let next_right = |cur| if is_front { prev(cur) } else { next(cur) };

        let new_left_vert = next_left(left_vert);
        let mut left_slope =
            VertexSlope::from_verts(&vertices[left_vert], &vertices[new_left_vert], w_buffer);
        let mut left_end = vertices[new_left_vert].screen_coords[1];
        left_vert = new_left_vert;
        let new_right_vert = next_right(right_vert);
        let mut right_slope =
            VertexSlope::from_verts(&vertices[right_vert], &vertices[new_right_vert], w_buffer);
        let mut right_end = vertices[new_right_vert].screen_coords[1];
        right_vert = new_right_vert;

        // Defense-in-depth: screen coordinates are clamped at the viewport
        // transform (see `Viewport::screen_coords`), but clamp the scanline
        // range here too so a future regression can't index past the
        // 256x192 frame buffer.
        let y_start = vertices[start_vert].screen_coords[1].min(GPU::HEIGHT as u32);
        let y_end = vertices[end_vert].screen_coords[1].min(GPU::HEIGHT as u32);
        for y in y_start..y_end {
            // Find next vertex below current
            while y >= left_end {
                let new_left_vert = next_left(left_vert);
                left_slope = VertexSlope::from_verts(
                    &vertices[left_vert],
                    &vertices[new_left_vert],
                    w_buffer,
                );
                left_end = vertices[new_left_vert].screen_coords[1];
                left_vert = new_left_vert;
            }
            while y >= right_end {
                let new_right_vert = next_right(right_vert);
                right_slope = VertexSlope::from_verts(
                    &vertices[right_vert],
                    &vertices[new_right_vert],
                    w_buffer,
                );
                right_end = vertices[new_right_vert].screen_coords[1];
                right_vert = new_right_vert;
            }
            let x_start = left_slope.next_x() as usize;
            let x_end = right_slope.next_x() as usize;
            let (x_start, x_end) =
                if x_start > x_end { (x_end, x_start) } else { (x_start, x_end) };
            let w_start = left_slope.next_w() as u16;
            let w_end = right_slope.next_w() as u16;
            let num_steps = x_end - x_start;
            let mut color = ColorSlope::new(
                &left_slope.next_color(),
                &right_slope.next_color(),
                num_steps,
                w_start,
                w_end,
            );
            let mut s = PerspectiveSlope::new(
                left_slope.next_s(),
                right_slope.next_s(),
                num_steps,
                w_start,
                w_end,
            );
            let mut t = PerspectiveSlope::new(
                left_slope.next_t(),
                right_slope.next_t(),
                num_steps,
                w_start,
                w_end,
            );
            let mut depth =
                Slope::new(left_slope.next_depth(), right_slope.next_depth(), num_steps);

            for x in x_start..x_end {
                let y = y as usize;
                let depth_val = depth.next() as u32;
                let pixel = &mut frame_buffer[y * GPU::WIDTH + x];

                let vert_color = FrameBufferColor::new5(color.next(), polygon.attrs.alpha);
                let fb_color = &pixel.color;
                let poly_color =
                    blend(polygon, vert_color, s.next() as i32 >> 4, t.next() as i32 >> 4);
                if poly_color.a5() == 0 {
                    // Pixel is totally tranpsarent so not rendered
                } else if disp3dcnt.alpha_blending && fb_color.a5() != 0 && poly_color.a5() != 0x1F
                {
                    let poly_alpha = poly_color.a5() as u16;
                    let calc = |old, new| (old * (0x1F - poly_alpha) + new * (poly_alpha + 1)) / 32;
                    pixel.color = FrameBufferColor::new8(
                        Color::new6(
                            calc(fb_color.r6() as u16, poly_color.r6() as u16) as u8,
                            calc(fb_color.g6() as u16, poly_color.g6() as u16) as u8,
                            calc(fb_color.b6() as u16, poly_color.b6() as u16) as u8,
                        ),
                        std::cmp::max(fb_color.a, poly_color.a),
                    );
                    if polygon.attrs.set_depth_translucent {
                        pixel.depth = depth_val
                    }
                } else if depth_test(pixel.depth, depth_val) {
                    pixel.color = poly_color;
                    pixel.depth = depth_val;
                }
            }
        }
    }

    fn get_tex_color(vram: &VRAM, polygon: &Polygon, s: i32, t: i32) -> Option<FrameBufferColor> {
        let vram_offset = polygon.tex_params.vram_offset;
        let pal_offset = polygon.palette_base;
        let size = (polygon.tex_params.size_s as u32, polygon.tex_params.size_t as u32);
        let size_shift = (polygon.tex_params.size_s_shift, polygon.tex_params.size_t_shift);
        let mask = (size.0 - 1, size.1 - 1);
        // TODO: Avoid code repitition
        let s = if polygon.tex_params.repeat_s {
            let (original_s, mask) = (s as u32, mask.0);
            let s = original_s & mask;
            if polygon.tex_params.flip_s && (original_s >> size_shift.0) % 2 == 1 {
                s ^ mask
            } else {
                s
            }
        // TODO: Replace with clamp
        } else if s < 0 {
            0
        } else if s as u32 > size.0 {
            mask.0
        } else {
            s as u32
        } as usize;
        let t = if polygon.tex_params.repeat_t {
            let (original_t, mask) = (t as u32, mask.1);
            let t = original_t & mask;
            if polygon.tex_params.flip_t && (original_t >> size_shift.1) % 2 == 1 {
                t ^ mask
            } else {
                t
            }
        // TODO: Replace with clamp
        } else if t < 0 {
            0
        } else if t as u32 > size.1 {
            mask.1
        } else {
            t as u32
        } as usize;
        let texel = t * polygon.tex_params.size_s + s;
        let color0_transparent = polygon.tex_params.color0_transparent;

        // TODO: Remove code duplication
        match polygon.tex_params.format {
            TextureFormat::NoTexture => None,
            TextureFormat::A3I5 => Some({
                let byte = vram.get_textures::<u8>(vram_offset + texel);
                let palette_color = byte & 0x1F;
                let alpha = byte >> 5 & 0x7;
                let color = Color::from(
                    vram.get_textures_pal::<u16>(pal_offset + 2 * palette_color as usize),
                );
                FrameBufferColor::new5(color, alpha * 4 + alpha / 2)
            }),
            TextureFormat::Palette4 => Some({
                let palette_color =
                    vram.get_textures::<u8>(vram_offset + texel / 4) >> (2 * (texel % 4)) & 0x3;
                let color = Color::from(
                    vram.get_textures_pal::<u16>(pal_offset / 2 + 2 * palette_color as usize),
                );
                let alpha = if palette_color == 0 && color0_transparent { 0 } else { 0x1F };
                FrameBufferColor::new5(color, alpha)
            }),
            TextureFormat::Palette16 => Some({
                let palette_color =
                    vram.get_textures::<u8>(vram_offset + texel / 2) >> (4 * (texel % 2)) & 0xF;
                let color = Color::from(
                    vram.get_textures_pal::<u16>(pal_offset + 2 * palette_color as usize),
                );
                let alpha = if palette_color == 0 && color0_transparent { 0 } else { 0x1F };
                FrameBufferColor::new5(color, alpha)
            }),
            TextureFormat::Compressed => {
                let num_blocks_row = polygon.tex_params.size_s / 4;
                let block_start_addr = t / 4 * num_blocks_row + s / 4;
                let base_addr = vram_offset + 4 * block_start_addr;
                {
                    use std::sync::atomic::{AtomicUsize, Ordering};

                    // Compressed textures only live in VRAM slot 0 or slot 2
                    // (GBATEK "DS 3D Textures - Compressed Texture Format",
                    // `#ds3dtextureformats`). A block address landing outside
                    // those slots means the game configured an inconsistent
                    // texture base/size; render it as fully transparent rather
                    // than panicking.
                    static SLOT0: AtomicUsize = AtomicUsize::new(0);
                    static SLOT2: AtomicUsize = AtomicUsize::new(0);
                    let c = if base_addr < 128 * 0x400 {
                        SLOT0.fetch_add(1, Ordering::Relaxed)
                    } else {
                        SLOT2.fetch_add(1, Ordering::Relaxed)
                    };
                    if c % 100_000 == 0 {
                        eprintln!(
                            "[dbg-ctex] slot0={} slot2={} base=0x{base_addr:X}",
                            SLOT0.load(Ordering::Relaxed),
                            SLOT2.load(Ordering::Relaxed)
                        );
                    }
                }
                let te = vram.get_textures::<u8>(base_addr + t % 4);
                let texel_val = te >> (2 * (s % 4)) & 0x3;
                Some({
                    // GBATEK "DS 3D Texture Formats - 4x4-Texel Compressed"
                    // (`#ds3dtextureformats`): the 2-bit texel data lives in
                    // texture slot 0 or 2, and each 4x4 block's 16-bit extra
                    // palette entry lives in slot 1, at
                    // `0x20000 + (texel_offset & 0x1FFFF) / 2` for slot 0 and
                    // a further `0x10000` higher for slot 2.
                    //
                    // The slot-2 displacement is 0x10000, i.e. the second
                    // half of slot 1. A smaller value makes every slot-2
                    // compressed texture read its palette entries from the
                    // wrong place, which decodes whole surfaces to the wrong
                    // colours or to the transparent entry, so geometry that
                    // uses them (large textured scenery such as buildings)
                    // disappears while geometry using other texture formats
                    // still draws.
                    let extra_palette_addr = (base_addr & 0x1_FFFF) / 2
                        + if base_addr < 128 * 0x400 {
                            0 // Slot 0
                        } else {
                            0x10000
                        }; // Slot 2
                    let extra_palette_info =
                        vram.get_textures::<u16>(128 * 0x400 + extra_palette_addr);
                    let mode = (extra_palette_info >> 14) & 0x3;
                    let pal_offset = pal_offset + 4 * (extra_palette_info & 0x3FFF) as usize;
                    let color = |num: u8| {
                        FrameBufferColor::new5(
                            Color::from(
                                vram.get_textures_pal::<u16>(pal_offset + 2 * num as usize),
                            ),
                            0x1F,
                        )
                    };
                    match mode {
                        0 => match texel_val {
                            0..=2 => color(texel_val),
                            3 => FrameBufferColor::new5(Color::new5(0, 0, 0), 0), // Transparent
                            _ => unreachable!(),
                        },
                        1 => match texel_val {
                            0 | 1 => color(texel_val),
                            2 => Self::combine_colors5(color(0), color(1), |val0, val1| {
                                (val0 + val1) / 2
                            }),
                            3 => FrameBufferColor::new5(Color::new5(0, 0, 0), 0), // Transparent
                            _ => unreachable!(),
                        },
                        2 => color(texel_val),
                        3 => match texel_val {
                            0 | 1 => color(texel_val),
                            2 => Self::combine_colors5(color(0), color(1), |val0, val1| {
                                (val0 * 5 + val1 * 3) / 8
                            }),
                            3 => Self::combine_colors5(color(0), color(1), |val0, val1| {
                                (val0 * 3 + val1 * 5) / 8
                            }),
                            _ => unreachable!(),
                        },
                        _ => unreachable!(),
                    }
                })
            }
            TextureFormat::A5I3 => Some({
                let byte = vram.get_textures::<u8>(vram_offset + texel);
                let palette_color = byte & 0x7;
                let alpha = byte >> 3 & 0x1F;
                let color = Color::from(
                    vram.get_textures_pal::<u16>(pal_offset + 2 * palette_color as usize),
                );
                FrameBufferColor::new5(color, alpha)
            }),
            TextureFormat::Palette256 => Some({
                let palette_color = vram.get_textures::<u8>(vram_offset + texel);
                let color = Color::from(
                    vram.get_textures_pal::<u16>(pal_offset + 2 * palette_color as usize),
                );
                let alpha = if palette_color == 0 && color0_transparent { 0 } else { 0x1F };
                FrameBufferColor::new5(color, alpha)
            }),
            TextureFormat::DirectColor => Some({
                let color_val = vram.get_textures::<u16>(vram_offset + 2 * texel);
                let alpha = if color_val & 0x8000 != 0 { 0x1F } else { 0 };
                FrameBufferColor::new5(Color::from(color_val), alpha)
            }),
        }
    }

    fn blend_tex<C, A>(
        tex_color: Option<FrameBufferColor>,
        vert_color: FrameBufferColor,
        color_f: C,
        alpha_f: A,
    ) -> FrameBufferColor
    where
        C: Fn(u16, u16) -> u16,
        A: Fn(u16, u16) -> u16,
    {
        if let Some(tex_color) = tex_color {
            FrameBufferColor::new6(
                Color::new6(
                    color_f(tex_color.r6() as u16, vert_color.r6() as u16) as u8,
                    color_f(tex_color.g6() as u16, vert_color.g6() as u16) as u8,
                    color_f(tex_color.b6() as u16, vert_color.b6() as u16) as u8,
                ),
                alpha_f(tex_color.a6() as u16, vert_color.a6() as u16) as u8,
            )
        } else {
            vert_color
        }
    }

    // TODO: Remove with const generics
    fn combine_colors5<F>(
        color_a: FrameBufferColor,
        color_b: FrameBufferColor,
        f: F,
    ) -> FrameBufferColor
    where
        F: Fn(u16, u16) -> u16,
    {
        assert_eq!(color_a.a, color_b.a);
        FrameBufferColor::new8(
            Color::new5(
                f(color_a.color.r5() as u16, color_b.color.r5() as u16) as u8,
                f(color_a.color.g5() as u16, color_b.color.g5() as u16) as u8,
                f(color_a.color.b5() as u16, color_b.color.b5() as u16) as u8,
            ),
            color_a.a,
        )
    }

    fn get_depth_test(polygon: &Polygon) -> fn(u32, u32) -> bool {
        // TODO: Account for special cases
        fn eq_depth_test(cur_depth: u32, new_depth: u32) -> bool {
            // `cur_depth` is a 24-bit value that can be smaller than 0x200
            // for geometry very close to the camera (a common case for a
            // close-up cutscene model); a plain `cur_depth - 0x200` then
            // underflows the `u32` and wraps to a huge value, which makes
            // the equal-depth test spuriously fail for exactly that
            // close-up geometry. Saturate instead of wrapping.
            new_depth >= cur_depth.saturating_sub(0x200)
                && new_depth <= cur_depth.saturating_add(0x200)
        }
        fn lt_depth_test(cur_depth: u32, new_depth: u32) -> bool {
            new_depth < cur_depth
        }
        if polygon.attrs.depth_test_eq { eq_depth_test } else { lt_depth_test }
    }
}

struct VertexSlope {
    x: FPSlope,
    w: Slope,
    s: PerspectiveSlope,
    t: PerspectiveSlope,
    depth: Slope,
    color: ColorSlope,
}

// TODO: RE slopes
impl VertexSlope {
    /// `w_buffer` selects the depth source per GBATEK "DS 3D Polygon List
    /// Commands - SwapBuffers" (`#ds3dpolygonlistcommands`): Z-buffering
    /// interpolates the normalized 24-bit `z_depth`, W-buffering
    /// interpolates the raw clip-space W linearly (no perspective
    /// correction is applied to depth in W mode).
    pub fn from_verts(start: &Vertex, end: &Vertex, w_buffer: bool) -> VertexSlope {
        let num_steps = (end.screen_coords[1] - start.screen_coords[1]) as usize;
        let w_start = start.normalized_w;
        let w_end = end.normalized_w;
        let (depth_start, depth_end) = if w_buffer {
            // Clip-space W is the depth value in W-buffer mode. It is read
            // straight from the vertex rather than cached in a separate
            // field, so `Vertex`'s savestate layout stays unchanged.
            (start.clip_coords[3].raw().max(1) as f32, end.clip_coords[3].raw().max(1) as f32)
        } else {
            (start.z_depth as f32, end.z_depth as f32)
        };
        VertexSlope {
            x: FPSlope::new(start.screen_coords[0], end.screen_coords[0], num_steps),
            w: Slope::new(w_start as f32, w_end as f32, num_steps),
            s: PerspectiveSlope::new(
                start.tex_coord[0] as f32,
                end.tex_coord[0] as f32,
                num_steps,
                w_start,
                w_end,
            ),
            t: PerspectiveSlope::new(
                start.tex_coord[1] as f32,
                end.tex_coord[1] as f32,
                num_steps,
                w_start,
                w_end,
            ),
            depth: Slope::new(depth_start, depth_end, num_steps),
            color: ColorSlope::new(&start.color, &end.color, num_steps, w_start, w_end),
        }
    }

    pub fn next_x(&mut self) -> u32 {
        self.x.next().clamp(0, GPU::WIDTH as u32 - 1)
    }

    pub fn next_w(&mut self) -> f32 {
        self.w.next()
    }

    pub fn next_s(&mut self) -> f32 {
        self.s.next()
    }

    pub fn next_t(&mut self) -> f32 {
        self.t.next()
    }

    pub fn next_depth(&mut self) -> f32 {
        self.depth.next()
    }

    pub fn next_color(&mut self) -> Color {
        self.color.next()
    }
}

struct ColorSlope {
    r: PerspectiveSlope,
    g: PerspectiveSlope,
    b: PerspectiveSlope,
}

impl ColorSlope {
    pub fn new(
        start_color: &Color,
        end_color: &Color,
        num_steps: usize,
        w_start: u16,
        w_end: u16,
    ) -> Self {
        ColorSlope {
            r: PerspectiveSlope::new(
                start_color.r8() as f32,
                end_color.r8() as f32,
                num_steps,
                w_start,
                w_end,
            ),
            g: PerspectiveSlope::new(
                start_color.g8() as f32,
                end_color.g8() as f32,
                num_steps,
                w_start,
                w_end,
            ),
            b: PerspectiveSlope::new(
                start_color.b8() as f32,
                end_color.b8() as f32,
                num_steps,
                w_start,
                w_end,
            ),
        }
    }

    pub fn next(&mut self) -> Color {
        Color::new8(self.r.next() as u8, self.g.next() as u8, self.b.next() as u8)
    }
}

struct PerspectiveSlope {
    cur: usize,
    start: f32,
    diff: f32,
    num_steps: f32,
    w_start: f32,
    w_end: f32,
}

impl PerspectiveSlope {
    pub fn new(start: f32, end: f32, num_steps: usize, w_start: u16, w_end: u16) -> Self {
        PerspectiveSlope {
            cur: 0,
            start,
            diff: end - start,
            num_steps: num_steps as f32,
            w_start: w_start as f32,
            w_end: w_end as f32,
        }
    }

    pub fn next(&mut self) -> f32 {
        let factor_fn = |cur| {
            (cur * self.w_start) / (((self.num_steps - cur) * self.w_end) + (cur * self.w_start))
        };
        let factor = (factor_fn)(self.cur as f32);
        // Hardware falls back to linear interpolation when the perspective
        // factor is degenerate (equal W values give a 0/0 denominator; a
        // zero-length span gives num_steps == 0). Without this guard a
        // single scanline can receive a NaN/±inf factor, which manifests
        // as a garbled horizontal line across an otherwise-correct polygon.
        let factor = if factor.is_finite() && self.num_steps > 0.0 {
            factor
        } else if self.num_steps > 0.0 {
            self.cur as f32 / self.num_steps
        } else {
            0.0
        };
        self.cur += 1;
        self.start + factor * self.diff
    }
}

#[derive(Debug)]
struct Slope {
    cur: f32,
    step: f32,
}

impl Slope {
    pub fn new(start: f32, end: f32, num_steps: usize) -> Self {
        Slope { cur: start, step: (end - start) / num_steps as f32 }
    }

    pub fn next(&mut self) -> f32 {
        let return_val = self.cur;
        self.cur += self.step;
        return_val
    }
}

pub struct FPSlope {
    cur: Frac<18>,
    step: Frac<18>,
    neg: bool,
}

impl FPSlope {
    pub fn new(start: u32, end: u32, num_steps: usize) -> Self {
        let neg = start > end;
        let num_steps = num_steps as u32;
        let diff = if neg { start - end } else { end - start };
        let x_major = diff > num_steps;
        FPSlope {
            cur: Frac::new(start)
                + if x_major { Frac(Frac::<18>::one().0 / 2) } else { Frac::zero() },
            step: if num_steps == 0 {
                Frac::zero()
            } else if num_steps == diff {
                Frac::one()
            } else {
                let recip: Frac<18> = Frac(Frac::<18>::one().0 / num_steps);
                Frac(diff * recip.0)
            },
            neg,
        }
    }

    pub fn next(&mut self) -> u32 {
        if self.neg {
            // TODO: Implement trait
            if self.step.0 > self.cur.0 {
                self.cur = Frac::zero()
            } else {
                self.cur -= self.step
            };
            self.cur.num()
        } else {
            let return_val = self.cur.num();
            self.cur += self.step;
            return_val
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Frac<const N: u8>(u32);

impl<const N: u8> Frac<N> {
    pub fn new(num: u32) -> Self {
        Frac(num << N)
    }
    pub fn num(&self) -> u32 {
        self.0 >> N
    }
    pub fn zero() -> Self {
        Frac(0)
    }
    pub fn one() -> Self {
        Frac(1 << N)
    }
}

impl<const N: u8> std::ops::Add<Frac<N>> for Frac<N> {
    type Output = Self;

    fn add(self, rhs: Frac<N>) -> Self::Output {
        Frac(self.0 + rhs.0)
    }
}

impl<const N: u8, const M: u8> std::ops::AddAssign<Frac<M>> for Frac<N> {
    fn add_assign(&mut self, rhs: Frac<M>) {
        if M > N {
            let lhs = self.0 << (M - N);
            self.0 = (lhs + rhs.0) >> (M - N);
        } else {
            let rhs = rhs.0 << (N - M);
            self.0 += rhs;
        };
    }
}

impl<const N: u8, const M: u8> std::ops::SubAssign<Frac<M>> for Frac<N> {
    fn sub_assign(&mut self, rhs: Frac<M>) {
        if M > N {
            let lhs = self.0 << (M - N);
            self.0 = (lhs - rhs.0) >> (M - N);
        } else {
            let rhs = rhs.0 << (N - M);
            self.0 -= rhs;
        };
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
pub struct FrameBufferPixel {
    color: FrameBufferColor,
    depth: u32,
}

impl FrameBufferPixel {
    pub fn new() -> Self {
        FrameBufferPixel { color: FrameBufferColor::new5(Color::new5(0, 0, 0), 0), depth: 0 }
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
struct FrameBufferColor {
    color: Color,
    a: u8,
}

impl FrameBufferColor {
    pub fn new5(color: Color, a: u8) -> Self {
        FrameBufferColor { color, a: Color::upscale::<3>(a) }
    }

    pub fn new6(color: Color, a: u8) -> Self {
        FrameBufferColor { color, a: Color::upscale::<2>(a) }
    }

    pub fn new8(color: Color, a: u8) -> Self {
        FrameBufferColor { color, a }
    }

    pub fn r5(&self) -> u8 {
        self.color.r5()
    }
    pub fn a5(&self) -> u8 {
        self.a >> 3
    }
    pub fn r6(&self) -> u8 {
        self.color.r6()
    }
    pub fn g6(&self) -> u8 {
        self.color.g6()
    }
    pub fn b6(&self) -> u8 {
        self.color.b6()
    }
    pub fn a6(&self) -> u8 {
        self.a >> 2
    }

    // TODO: Convert 2D engine to also use 8 bit color
    pub fn as_u16(&self) -> u16 {
        self.color.as_u16() | if self.a == 0 { 0 } else { 0x8000 }
    }
}
