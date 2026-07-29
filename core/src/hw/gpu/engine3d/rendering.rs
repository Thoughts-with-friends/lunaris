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

        if self.attr_buffer.len() != self.frame_buffer.len() {
            // A savestate saved before `attr_buffer` existed does not resize
            // it, since the field is skipped entirely; recreate it here so
            // an old state still renders.
            self.attr_buffer = vec![FrameBufferAttr::default(); self.frame_buffer.len()];
        }

        // TODO: Optimize
        // GBATEK "DS 3D Toon, Edge, Fog, Alpha Blending, Anti-aliasing":
        // CLEAR_COLOR bit 15 seeds the fog flag for pixels the rear plane
        // still shows through, exactly like an opaque polygon's POLYGON_ATTR
        // bit 15 does for a drawn pixel.
        for (pixel, attr) in self.frame_buffer.iter_mut().zip(self.attr_buffer.iter_mut()) {
            pixel.color = FrameBufferColor::new5(
                Color::new5(self.clear_color.r, self.clear_color.g, self.clear_color.b),
                self.clear_color.a,
            );
            pixel.depth = self.clear_depth.depth();
            *attr = FrameBufferAttr {
                fog: self.clear_color.fog,
                opaque_id: self.clear_color.polygon_id,
                translucent_id: None,
            };
        }

        let w_buffer = self.frame_params.w_buffer;
        // Alpha test (DISP3DCNT bit 2 + ALPHA_TEST_REF) is applied per fragment
        // in `render_polygon`; see `docs/design/rendering-audio-fix-design.md`
        // item F-1.
        let alpha_test_ref = if self.disp3dcnt.alpha_test { self.alpha_test_ref } else { 0 };

        let disp3dcnt = &self.disp3dcnt;
        let toon_table = &self.toon_table;
        let blend = |polygon: &Polygon, vert_color, s: i32, t: i32| {
            let tex_color = Self::get_tex_color(vram, polygon, s, t);
            let modulation_blend = |val1, val2| ((val1 + 1) * (val2 + 1) - 1) / 64;
            match polygon.attrs.mode {
                PolygonMode::Modulation => {
                    Self::blend_tex(tex_color, vert_color, modulation_blend, modulation_blend)
                }
                // GBATEK "DS 3D Texture Blending" (`#ds3dtextureblending`),
                // highlight shading: the modulated value has the vertex
                // component added on top and the sum is *clamped* to the 6-bit
                // maximum. Using `max` here instead of `min` forced every
                // highlight-shaded surface to at least 63 in all three
                // channels, i.e. flat white.
                PolygonMode::ToonHighlight if disp3dcnt.highlight_shading => Self::blend_tex(
                    tex_color,
                    vert_color,
                    |val1, val2| std::cmp::min(modulation_blend(val1, val2) + val2, 0x3F),
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
        let attr_buffer = &mut self.attr_buffer;
        let mut render = |polygon: Polygon| {
            let vertices = &vertices[polygon.start_vert..polygon.end_vert];
            Self::render_polygon(
                disp3dcnt,
                w_buffer,
                alpha_test_ref,
                blend,
                &polygon,
                vertices,
                frame_buffer,
                attr_buffer,
            );
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

        if self.disp3dcnt.fog_master_enable {
            self.apply_fog();
        }

        self.vertices.clear();
        self.gxstat.geometry_engine_busy = false;
        self.polygons_submitted = false;
    }

    /// Fog post-pass, run once over the whole frame buffer after every
    /// polygon has been rasterized. Blends `FOG_COLOR` into every pixel
    /// flagged for fog (see `render_polygon` and the clear loop in
    /// `render`), weighted by a density that ramps from `FOG_TABLE[0]` at
    /// `FOG_OFFSET` up to `FOG_TABLE[31]` by the end of the fog range set by
    /// DISP3DCNT's Fog Depth Shift.
    ///
    /// GBATEK "DS 3D Toon, Edge, Fog, Alpha Blending, Anti-aliasing":
    /// <https://problemkaputt.de/gbatek-ds-3d-toon-edge-fog-alpha-blending-anti-aliasing.htm>
    ///
    /// melonDS `GPU3D_Soft.cpp`: `CalculateFogDensity`, `ScanlineFinalPass`.
    /// See `docs/design/3d-fog-and-rendering-fixes-design.md` §4.
    fn apply_fog(&mut self) {
        // FOG_OFFSET is a 15-bit depth; the Z-buffer is 24-bit, so scale up
        // by the same 0x200 factor CLEAR_DEPTH uses (`ClearDepth::depth`).
        let fog_offset = self.fog_offset as u32 * 0x200;
        let fog_shift = self.disp3dcnt.fog_depth_shift;

        // Density lookup interpolates between table entries `n` and `n+1`,
        // and `n` can reach 32, so pad the raw 32-entry table on both ends
        // rather than indexing past its bounds.
        let mut padded = [0u8; 34];
        padded[0] = self.fog_table[0];
        padded[1..=32].copy_from_slice(&self.fog_table);
        padded[33] = self.fog_table[31];

        let fog_color = FrameBufferColor::new5(
            Color::new5(self.fog_color.r, self.fog_color.g, self.fog_color.b),
            self.fog_color.a,
        );
        let fog_alpha_only = self.disp3dcnt.fog_alpha_only;

        for (pixel, attr) in self.frame_buffer.iter_mut().zip(self.attr_buffer.iter()) {
            if !attr.fog {
                continue;
            }
            let density = Self::fog_density(pixel.depth, fog_offset, fog_shift, &padded);
            let blend = |old: u32, new: u32| (new * density + old * (128 - density)) >> 7;
            let color = if fog_alpha_only {
                pixel.color.color
            } else {
                Color::new6(
                    blend(pixel.color.r6() as u32, fog_color.r6() as u32) as u8,
                    blend(pixel.color.g6() as u32, fog_color.g6() as u32) as u8,
                    blend(pixel.color.b6() as u32, fog_color.b6() as u32) as u8,
                )
            };
            let alpha = blend(pixel.color.a6() as u32, fog_color.a6() as u32) as u8;
            pixel.color = FrameBufferColor::new6(color, alpha);
        }
    }

    /// Computes the 0..=128 fog density (blend weight, out of 128) for a
    /// 24-bit depth value. `padded` is the 34-entry table built by
    /// `apply_fog`. See `docs/design/3d-fog-and-rendering-fixes-design.md`
    /// §4.3 and melonDS `GPU3D_Soft.cpp::CalculateFogDensity`.
    fn fog_density(depth: u32, fog_offset: u32, fog_shift: u8, padded: &[u8; 34]) -> u32 {
        let (density_id, density_frac) = if depth < fog_offset {
            (0, 0)
        } else {
            // Z difference is shifted right by 2, then left by the fog
            // shift; bits 0-16 of the result are the interpolation
            // fraction, bits 17+ are the table index. Hardware lets this
            // wrap on a large enough shift, so use wrapping arithmetic
            // rather than panicking on overflow in debug builds.
            let z = ((depth - fog_offset) >> 2).wrapping_shl(fog_shift as u32);
            let density_id = z >> 17;
            if density_id >= 32 { (32, 0) } else { (density_id, z & 0x1_FFFF) }
        };

        let density = (padded[density_id as usize] as u32 * (0x2_0000 - density_frac)
            + padded[density_id as usize + 1] as u32 * density_frac)
            >> 17;
        if density >= 127 { 128 } else { density }
    }

    /// Rasterizes one polygon.
    ///
    /// `alpha_test_ref` is the ALPHA_TEST_REF value when DISP3DCNT bit 2 is
    /// set, and 0 when the alpha test is disabled: a fragment is discarded
    /// when its 5-bit alpha is less than or equal to it, which also covers
    /// the always-applied "alpha 0 is invisible" rule.
    ///
    /// GBATEK "DS 3D Display Control": <https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol>
    fn render_polygon<B>(
        disp3dcnt: &DISP3DCNT,
        w_buffer: bool,
        alpha_test_ref: u8,
        blend: B,
        polygon: &Polygon,
        vertices: &[Vertex],
        frame_buffer: &mut [FrameBufferPixel],
        attr_buffer: &mut [FrameBufferAttr],
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
                let index = y * GPU::WIDTH + x;
                let pixel = &mut frame_buffer[index];
                let attr = &mut attr_buffer[index];

                let vert_color = FrameBufferColor::new5(color.next(), polygon.attrs.alpha);
                let fb_color = &pixel.color;
                let poly_color =
                    blend(polygon, vert_color, s.next() as i32 >> 4, t.next() as i32 >> 4);
                if poly_color.a5() <= alpha_test_ref {
                    // Fragment failed the alpha test (or is fully transparent),
                    // so neither color, depth, nor attributes are written.
                } else if disp3dcnt.alpha_blending
                    && fb_color.a5() != 0
                    && poly_color.a5() != 0x1F
                    && depth_test(pixel.depth, depth_val)
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
                    // GBATEK "DS 3D Toon, Edge, Fog, Alpha Blending,
                    // Anti-aliasing"; melonDS `PlotTranslucentPixel`: a
                    // translucent write can only *clear* the destination's
                    // fog flag, never set it (the polygon's own fog flag is
                    // ANDed in, not OR'd).
                    attr.fog &= polygon.attrs.fog_enable;
                    attr.translucent_id = Some(polygon.attrs.polygon_id);
                } else if depth_test(pixel.depth, depth_val) {
                    pixel.color = poly_color;
                    pixel.depth = depth_val;
                    attr.fog = polygon.attrs.fog_enable;
                    attr.opaque_id = polygon.attrs.polygon_id;
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
        // Clamped (non-repeating) mode: the valid texel range is
        // `0 ..= size - 1`, so a coordinate equal to the texture dimension
        // must already clamp to the last texel instead of being used as an
        // index one column past the end of the texture.
        } else if s < 0 {
            0
        } else if s as u32 >= size.0 {
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
        // Same off-by-one as the S axis above.
        } else if t < 0 {
            0
        } else if t as u32 >= size.1 {
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

/// Per-pixel render attributes, rebuilt from scratch every frame alongside
/// `frame_buffer`. Not part of the savestate; see
/// `docs/design/3d-fog-and-rendering-fixes-design.md` §2.
#[derive(Clone, Copy, Default)]
pub struct FrameBufferAttr {
    /// Whether this pixel should be affected by the fog post-pass. Set from
    /// POLYGON_ATTR bit 15 on an opaque write or the rear-plane's fog flag on
    /// clear; a translucent write can only clear it (never set it), matching
    /// melonDS `PlotTranslucentPixel`.
    pub fog: bool,
    /// Polygon ID of the last opaque write, used by edge marking.
    pub opaque_id: u8,
    /// Polygon ID of the last translucent write, if any; used to reject
    /// blending a translucent polygon's own overlapping faces against
    /// themselves.
    pub translucent_id: Option<u8>,
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

#[cfg(test)]
mod fog_tests {
    use super::Engine3D;

    fn padded(raw: [u8; 32]) -> [u8; 34] {
        let mut padded = [0u8; 34];
        padded[0] = raw[0];
        padded[1..=32].copy_from_slice(&raw);
        padded[33] = raw[31];
        padded
    }

    /// A depth below FOG_OFFSET always reads FOG_TABLE[0] with no
    /// interpolation, per GBATEK/melonDS `CalculateFogDensity`.
    #[test]
    fn depth_below_offset_uses_first_table_entry_unblended() {
        let mut raw = [0u8; 32];
        raw[0] = 40;
        raw[1] = 100;
        let padded = padded(raw);
        assert_eq!(Engine3D::fog_density(0, 0x1000, 4, &padded), 40);
        assert_eq!(Engine3D::fog_density(0xFFF, 0x1000, 4, &padded), 40);
    }

    /// A density index that would reach 33 must clamp to 32 rather than
    /// indexing one entry past the padded table's end.
    #[test]
    fn density_id_clamps_at_the_end_of_the_table_without_panicking() {
        let mut raw = [0u8; 32];
        raw[31] = 77;
        let padded = padded(raw);
        // A huge depth pushes density_id well past 32; must not panic and
        // must saturate to the last table entry (77, unblended).
        let density = Engine3D::fog_density(u32::MAX, 0, 15, &padded);
        assert_eq!(density, 77);
    }

    /// Per melonDS: `if (density >= 127) density = 128;` -- not `min(_, 127)`.
    /// Only a raw density of exactly 128 (achieved by rounding up from 127)
    /// produces the fully-saturated `>> 7` blend weight.
    #[test]
    fn density_of_127_rounds_up_to_128_not_clamped_to_127() {
        let raw = [127u8; 32];
        let padded = padded(raw);
        assert_eq!(Engine3D::fog_density(0x10000, 0, 0, &padded), 128);
    }

    /// FOG_TABLE writes mask to 7 bits and FOG_OFFSET writes mask to 15
    /// bits; exercised through the same padding path unit tests use.
    #[test]
    fn padded_table_repeats_first_and_last_entries() {
        let mut raw = [0u8; 32];
        raw[0] = 5;
        raw[31] = 9;
        let padded = padded(raw);
        assert_eq!(padded[0], 5);
        assert_eq!(padded[1], 5);
        assert_eq!(padded[32], 9);
        assert_eq!(padded[33], 9);
    }
}
