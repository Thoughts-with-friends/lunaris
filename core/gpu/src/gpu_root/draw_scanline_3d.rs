use crate::gpu_root::Gpu;
use lunaris_ds_mem_const::PIXELS_PER_LINE;

impl Gpu {
    /// NOTE: 3D engine method
    pub fn render_scanline(&mut self, is_engine_a: bool, bg0_priority: u8) {
        let line = self.get_vcount() as usize;

        // Rear plane
        let rear_z = (self.engine_3d.clear_depth * 0x200)
            + (((self.engine_3d.clear_depth + 1) / 0x8000) * 0x1FF);

        for i in 0..PIXELS_PER_LINE {
            self.engine_3d.z_buffer[line][i] = rear_z;
            self.engine_3d.trans_poly_ids[i] = 0xFF;
        }

        let y_coord = line * PIXELS_PER_LINE;

        for poly_i in 0..self.engine_3d.rend_poly_count as usize {
            let poly = &self.engine_3d.rend_poly[poly_i];

            if line < poly.top_y as usize || line > poly.bottom_y as usize {
                continue;
            }

            if poly.attributes.polygon_mode == 3 {
                continue; //TODO: shadow polygons
            }

            // --- edge scan state ---
            let mut left_x: i32 = 512;
            let mut right_x: i32 = -512;

            let mut left_r = 0u32;
            let mut left_g = 0u32;
            let mut left_b = 0u32;

            let mut right_r = 0u32;
            let mut right_g = 0u32;
            let mut right_b = 0u32;

            let mut left_z = 0_i64;
            let mut right_z = 0_i64;

            let mut left_w = 0i32;
            let mut right_w = 0i32;

            let mut left_s = 0i16;
            let mut left_t = 0i16;
            let mut right_s = 0i16;
            let mut right_t = 0i16;

            // NOTE: vert_pointer
            let vert_base = poly.vert_index as usize;

            // =================================================
            // EDGE WALK (Bresenham-ish)
            // =================================================
            for vert in 0..poly.vertices as usize {
                let next = (vert + 1) % poly.vertices as usize;

                let v1 = &self.engine_3d.rend_vert[vert_base + vert];
                let v2 = &self.engine_3d.rend_vert[vert_base + next];

                let mut x1 = v1.coords[0];
                let mut y1 = v1.coords[1];
                let mut x2 = v2.coords[0];
                let mut y2 = v2.coords[1];

                let mut r1 = v1.final_colors[0] as i64;
                let mut g1 = v1.final_colors[1] as i64;
                let mut b1 = v1.final_colors[2] as i64;

                let mut r2 = v2.final_colors[0] as i64;
                let mut g2 = v2.final_colors[1] as i64;
                let mut b2 = v2.final_colors[2] as i64;

                let mut z1 = v1.coords[2] as i64;
                let mut z2 = v2.coords[2] as i64;

                let mut w1 = v1.coords[3];
                let mut w2 = v2.coords[3];

                let mut s1 = v1.texcoords[0] as i64;
                let mut s2 = v2.texcoords[0] as i64;
                let mut t1 = v1.texcoords[1] as i64;
                let mut t2 = v2.texcoords[1] as i64;

                let steep = (y2 - y1).abs() > (x2 - x1).abs();

                if steep {
                    std::mem::swap(&mut x1, &mut y1);
                    std::mem::swap(&mut x2, &mut y2);
                }

                if x1 > x2 {
                    std::mem::swap(&mut x1, &mut x2);
                    std::mem::swap(&mut y1, &mut y2);
                    std::mem::swap(&mut r1, &mut r2);
                    std::mem::swap(&mut g1, &mut g2);
                    std::mem::swap(&mut b1, &mut b2);
                    std::mem::swap(&mut z1, &mut z2);
                    std::mem::swap(&mut w1, &mut w2);
                    std::mem::swap(&mut s1, &mut s2);
                    std::mem::swap(&mut t1, &mut t2);
                }

                let dx = x2 - x1;
                let dy = (y2 - y1).abs();

                let mut error = dx / 2;
                let y_step = if y1 < y2 { 1 } else { -1 };

                let mut y = y1;

                let mut left_pixel = -1;
                let mut right_pixel = -1;

                for x in x1..=x2 {
                    if steep {
                        if line as i32 == x {
                            if y < left_x {
                                left_x = y;
                                left_pixel = x;
                            }
                            if y > right_x {
                                right_x = y;
                                right_pixel = x;
                            }
                        }
                    } else if line as i32 == y {
                        if x < left_x {
                            left_x = x;
                            left_pixel = x;
                        }
                        if x > right_x {
                            right_x = x;
                            right_pixel = x;
                        }
                    }

                    error -= dy;
                    if error < 0 {
                        y += y_step;
                        error += dx;
                    }
                }

                // --- interpolate edges ---
                if left_pixel >= 0 {
                    let len = (x2 - x1 + 1) as u64;
                    let pos = (left_pixel - x1) as u64;

                    left_r = self.engine_3d.interpolate(pos, len, r1, r2, w1, w2) as u32;
                    left_g = self.engine_3d.interpolate(pos, len, g1, g2, w1, w2) as u32;
                    left_b = self.engine_3d.interpolate(pos, len, b1, b2, w1, w2) as u32;
                    left_z = self.engine_3d.interpolate(pos, len, z1, z2, w1, w2);

                    left_w = {
                        let w_w1 = w1 as i64;
                        let w_w2 = w2 as i64;
                        self.engine_3d.interpolate(pos, len, w_w1, w_w2, w1, w2) as i32
                    };
                    left_s = self.engine_3d.interpolate(pos, len, s1, s2, w1, w2) as i16;
                    left_t = self.engine_3d.interpolate(pos, len, t1, t2, w1, w2) as i16;
                }

                if right_pixel >= 0 {
                    let len = (x2 - x1 + 1) as u64;
                    let pos = (right_pixel - x1) as u64;

                    right_r = self.engine_3d.interpolate(pos, len, r1, r2, w1, w2) as u32;
                    right_g = self.engine_3d.interpolate(pos, len, g1, g2, w1, w2) as u32;
                    right_b = self.engine_3d.interpolate(pos, len, b1, b2, w1, w2) as u32;
                    right_z = self.engine_3d.interpolate(pos, len, z1, z2, w1, w2);

                    right_w = {
                        let w_w1 = w1 as i64;
                        let w_w2 = w2 as i64;
                        self.engine_3d.interpolate(pos, len, w_w1, w_w2, w1, w2) as i32
                    };
                    right_s = self.engine_3d.interpolate(pos, len, s1, s2, w1, w2) as i16;
                    right_t = self.engine_3d.interpolate(pos, len, t1, t2, w1, w2) as i16;
                }
            }

            // =================================================
            // SPAN FILL
            // =================================================

            let left_x = left_x.max(0);
            let right_x = right_x.min((PIXELS_PER_LINE - 1) as i32);

            if right_x < left_x {
                continue;
            }

            let line_len = (right_x - left_x + 1) as u64;

            // Calculate texture stuff in advance
            let texparams = poly.texparams.clone();
            let texture_mapping = self.engine_3d.disp3dcnt.texture_mapping && texparams.format != 0;

            let tex_width = 8 << texparams.s_size;
            let tex_height = 8 << texparams.t_size;
            let tex_vram_offset = texparams.vram_offset * 8;

            // Fill the polygon
            for x in left_x..=right_x {
                let pix_pos = (x - left_x) as u64;

                let pix_z = self
                    .engine_3d
                    .interpolate(pix_pos, line_len, left_z, right_z, left_w, right_w)
                    as u32;

                let x_us = x as usize;

                if poly.attributes.depth_test_equal {
                    let low_z = self.engine_3d.z_buffer[line][x_us] - 0x200;
                    let high_z = self.engine_3d.z_buffer[line][x_us] + 0x200;
                    if pix_z < low_z || pix_z > high_z {
                        continue;
                    }
                } else if pix_z > self.engine_3d.z_buffer[line][x_us] {
                    continue;
                }

                let mut vr: u32;
                let mut vg: u32;
                let mut vb: u32;

                let mut tr: u32 = 0x3E;
                let mut tg: u32 = 0x3E;
                let mut tb: u32 = 0x3E;
                let mut ta: u32 = 0x1F;

                // ===== wireframe / alpha =====
                let va: u32 = if poly.attributes.alpha == 0 {
                    if x == left_x || x == right_x {
                        0x1F
                    } else {
                        continue;
                    }
                } else {
                    poly.attributes.alpha as u32
                };

                // ===== vertex color interpolate =====
                vr = (self.engine_3d.interpolate(
                    pix_pos,
                    line_len,
                    left_r as i64,
                    right_r as i64,
                    left_w,
                    right_w,
                ) >> 4) as u32;
                vg = (self.engine_3d.interpolate(
                    pix_pos,
                    line_len,
                    left_g as i64,
                    right_g as i64,
                    left_w,
                    right_w,
                ) >> 4) as u32;
                vb = (self.engine_3d.interpolate(
                    pix_pos,
                    line_len,
                    left_b as i64,
                    right_b as i64,
                    left_w,
                    right_w,
                ) >> 4) as u32;

                vr <<= 1;
                vg <<= 1;
                vb <<= 1;

                if vr != 0 {
                    vr += 1;
                }
                if vg != 0 {
                    vg += 1;
                }
                if vb != 0 {
                    vb += 1;
                }

                // =================================================
                // TEXTURE
                // =================================================
                if texture_mapping {
                    let mut s = (self.engine_3d.interpolate(
                        pix_pos,
                        line_len,
                        left_s as i64,
                        right_s as i64,
                        left_w,
                        right_w,
                    ) >> 4) as i32;
                    let mut t = (self.engine_3d.interpolate(
                        pix_pos,
                        line_len,
                        left_t as i64,
                        right_t as i64,
                        left_w,
                        right_w,
                    ) >> 4) as i32;

                    // ---- clamp / repeat S ----
                    if !texparams.repeat_s {
                        if s < 0 {
                            s = 0;
                        }
                        if s >= tex_width {
                            s = tex_width - 1;
                        }
                    } else if texparams.flip_s && (s & tex_width) != 0 {
                        // Repeat
                        s = (tex_width - 1) - (s & (tex_width - 1));
                    } else {
                        s &= tex_width - 1;
                    }

                    // ---- clamp / repeat T ----
                    if !texparams.repeat_t {
                        if t < 0 {
                            t = 0;
                        }
                        if t >= tex_height {
                            t = tex_height - 1;
                        }
                    } else if texparams.flip_t && (t & tex_height) != 0 {
                        t = (tex_height - 1) - (t & (tex_height - 1));
                    } else {
                        t &= tex_height - 1;
                    }

                    match texparams.format {
                        1 => {
                            // A3I5
                            let mut texel_addr = s as u32;
                            texel_addr += (t as u32) * tex_width as u32;
                            texel_addr += tex_vram_offset as u32;

                            let data = self.read_teximage_u8(texel_addr);

                            let color_index = data & 0x1F;
                            let alpha_index = data >> 5;

                            ta = ((alpha_index << 2) + (alpha_index >> 1)) as u32;

                            if color_index != 0 || !texparams.color0_transparent {
                                let mut pal_addr = (color_index as u32) * 2;
                                pal_addr += poly.palette_base * 0x10;

                                let pal_color = self.read_texpal_u16(pal_addr);

                                tr = ((pal_color & 0x1F) << 1) as u32;
                                tg = (((pal_color >> 5) & 0x1F) << 1) as u32;
                                tb = (((pal_color >> 10) & 0x1F) << 1) as u32;
                            } else {
                                ta = 0;
                            }
                        }

                        2 => {
                            // 4 color palette
                            let mut texel_addr = s as u32;
                            texel_addr += (t as u32) * tex_width as u32;
                            texel_addr /= 4;
                            texel_addr += tex_vram_offset as u32;

                            let mut data = self.read_teximage_u8(texel_addr);
                            data = (data >> ((texel_addr & 0x3) * 2)) & 0x3;

                            if data != 0 || !texparams.color0_transparent {
                                let mut pal_addr = (data as u32) * 2;
                                pal_addr += poly.palette_base * 0x8;

                                let pal_color = self.read_texpal_u16(pal_addr);

                                tr = ((pal_color & 0x1F) << 1) as u32;
                                tg = (((pal_color >> 5) & 0x1F) << 1) as u32;
                                tb = (((pal_color >> 10) & 0x1F) << 1) as u32;
                            } else {
                                ta = 0;
                            }
                        }

                        3 => {
                            // 16 color palette
                            let mut texel_addr = s as u32;
                            texel_addr += (t as u32) * tex_width as u32;
                            texel_addr /= 2;
                            texel_addr += tex_vram_offset as u32;

                            let mut data = self.read_teximage_u8(texel_addr);

                            if (s & 1) != 0 {
                                data >>= 4;
                            } else {
                                data &= 0xF;
                            }

                            if data != 0 || !texparams.color0_transparent {
                                let mut pal_addr = (data as u32) * 2;
                                pal_addr += poly.palette_base * 0x10;

                                let pal_color = self.read_texpal_u16(pal_addr);

                                tr = ((pal_color & 0x1F) << 1) as u32;
                                tg = (((pal_color >> 5) & 0x1F) << 1) as u32;
                                tb = (((pal_color >> 10) & 0x1F) << 1) as u32;
                            } else {
                                ta = 0;
                            }
                        }

                        4 => {
                            // 256 color palette
                            let mut texel_addr = s as u32;
                            texel_addr += (t as u32) * tex_width as u32;
                            texel_addr += tex_vram_offset as u32;

                            let data = self.read_teximage_u8(texel_addr);

                            if data != 0 || !texparams.color0_transparent {
                                let mut pal_addr = (data as u32) * 2;
                                pal_addr += poly.palette_base * 0x10;

                                let pal_color = self.read_texpal_u16(pal_addr);

                                tr = ((pal_color & 0x1F) << 1) as u32;
                                tg = (((pal_color >> 5) & 0x1F) << 1) as u32;
                                tb = (((pal_color >> 10) & 0x1F) << 1) as u32;
                            } else {
                                ta = 0;
                            }
                        }

                        5 => {
                            // Compressed 4x4
                            let mut texel_addr = (s as u32) & 0x3FC;
                            texel_addr += ((t as u32) & 0x3FC) * ((tex_width as u32) >> 2);
                            texel_addr += tex_vram_offset as u32;
                            texel_addr += (t as u32) & 0x3;

                            let mut slot1_addr = 0x20000 + ((texel_addr >> 1) & 0xFFFE);
                            if texel_addr >= 0x40000 {
                                slot1_addr += 0x10000;
                            }

                            let mut data = self.read_teximage_u8(texel_addr);
                            data >>= 2 * (s as u32 & 0x3);
                            data &= 0x3;

                            let palette_data = self.read_teximage_u16(slot1_addr);
                            let palette_offset = ((palette_data & 0x3FFF) as u32) << 2;

                            let palette_base = poly.palette_base * 0x10;

                            fn calc_rgb_color(
                                color0: u16,
                                color1: u16,
                                mul: u16,
                                shift: u16,
                            ) -> u16 {
                                let r0 = color0 & 0x1F;
                                let r1 = color1 & 0x1F;
                                let g0 = (color0 >> 5) & 0x1F;
                                let g1 = (color1 >> 5) & 0x1F;
                                let b0 = (color0 >> 10) & 0x1F;
                                let b1 = (color1 >> 10) & 0x1F;

                                let r = (r0 * 5 + r1 * mul) >> shift;
                                let g = (g0 * 5 + g1 * mul) >> shift;
                                let b = (b0 * 5 + b1 * mul) >> shift;
                                r | (g << 5) | (b << 10)
                            }

                            let color: u16 = match data {
                                0 => self.read_texpal_u16(palette_base + palette_offset),
                                1 => self.read_texpal_u16(palette_base + palette_offset + 2),
                                2 => match palette_data >> 14 {
                                    1 => {
                                        let color0 =
                                            self.read_texpal_u16(palette_base + palette_offset);
                                        let color1 =
                                            self.read_texpal_u16(palette_base + palette_offset + 2);
                                        calc_rgb_color(color0, color1, 1, 1)
                                    }
                                    3 => {
                                        let color0 =
                                            self.read_texpal_u16(palette_base + palette_offset);
                                        let color1 =
                                            self.read_texpal_u16(palette_base + palette_offset + 2);
                                        calc_rgb_color(color0, color1, 3, 3)
                                    }
                                    _ => self.read_texpal_u16(palette_base + palette_offset + 4),
                                },
                                3 => match palette_data >> 14 {
                                    2 => self.read_texpal_u16(palette_base + palette_offset + 6),
                                    3 => {
                                        let color0 =
                                            self.read_texpal_u16(palette_base + palette_offset);
                                        let color1 =
                                            self.read_texpal_u16(palette_base + palette_offset + 2);
                                        calc_rgb_color(color0, color1, 5, 3)
                                    }
                                    _ => {
                                        ta = 0;
                                        0
                                    }
                                },
                                _ => {
                                    #[cfg(feature = "tracing")]
                                    tracing::error!("Unrecognized 4x4 texel data type {data}");
                                    0
                                }
                            };

                            tr = ((color & 0x1F) << 1) as u32;
                            tg = (((color >> 5) & 0x1F) << 1) as u32;
                            tb = (((color >> 10) & 0x1F) << 1) as u32;
                        }

                        6 => {
                            // A5I3
                            let mut texel_addr = s as u32;
                            texel_addr += (t as u32) * tex_width as u32;
                            texel_addr += tex_vram_offset as u32;

                            let data = self.read_teximage_u8(texel_addr);

                            let color_index = data & 0x7;
                            ta = (data >> 3) as u32;

                            if color_index != 0 || !texparams.color0_transparent {
                                let mut pal_addr = (color_index as u32) * 2;
                                pal_addr += poly.palette_base * 0x10;

                                let pal_color = self.read_texpal_u16(pal_addr);

                                tr = ((pal_color & 0x1F) << 1) as u32;
                                tg = (((pal_color >> 5) & 0x1F) << 1) as u32;
                                tb = (((pal_color >> 10) & 0x1F) << 1) as u32;
                            } else {
                                ta = 0;
                            }
                        }

                        7 => {
                            // Direct color
                            let mut texel_addr = s as u32;
                            texel_addr += (t as u32) * tex_width as u32;
                            texel_addr *= 2;
                            texel_addr += tex_vram_offset as u32;

                            let data = self.read_teximage_u16(texel_addr);

                            if (data & (1 << 15)) != 0 {
                                tr = ((data & 0x1F) << 1) as u32;
                                tg = (((data >> 5) & 0x1F) << 1) as u32;
                                tb = (((data >> 10) & 0x1F) << 1) as u32;
                            } else {
                                ta = 0;
                            }
                        }

                        _ => {
                            #[cfg(feature = "tracing")]
                            tracing::error!("Unrecognized texture format {}", texparams.format);
                        }
                    }
                }

                // ===== texture normalize =====
                if tr != 0 {
                    tr += 1;
                }
                if tg != 0 {
                    tg += 1;
                }
                if tb != 0 {
                    tb += 1;
                }

                // =================================================
                // POLYGON MODE
                // =================================================
                let (mut r, mut g, mut b, alpha);

                match poly.attributes.polygon_mode {
                    0 => {
                        r = (((tr + 1) * (vr + 1) - 1) / 64) << 2;
                        g = (((tg + 1) * (vg + 1) - 1) / 64) << 2;
                        b = (((tb + 1) * (vb + 1) - 1) / 64) << 2;
                        alpha = ((ta + 1) * (va + 1) - 1) / 32;
                    }

                    2 => {
                        if self.engine_3d.disp3dcnt.highlight_shading {
                            vg = vr;
                            vb = vr;
                        } else {
                            let toon_color = self.engine_3d.toon_table[(vr >> 1) as usize];

                            vr = ((toon_color & 0x1F) << 1) as u32;
                            vg = (((toon_color >> 5) & 0x1F) << 1) as u32;
                            vb = (((toon_color >> 10) & 0x1F) << 1) as u32;

                            if vr != 0 {
                                vr += 1;
                            }
                            if vg != 0 {
                                vg += 1;
                            }
                            if vb != 0 {
                                vb += 1;
                            }
                        }

                        r = (((tr + 1) * (vr + 1) - 1) / 64) << 2;
                        g = (((tg + 1) * (vg + 1) - 1) / 64) << 2;
                        b = (((tb + 1) * (vb + 1) - 1) / 64) << 2;
                        alpha = ((ta + 1) * (va + 1) - 1) / 32;
                    }

                    unknown => {
                        #[cfg(feature = "tracing")]
                        tracing::error!("Unrecognized polygon rendering mode {unknown}");
                        continue;
                    }
                }

                if alpha == 0 {
                    continue;
                }

                // ===== Z write =====
                if !poly.translucent || poly.attributes.set_new_trans_depth {
                    self.engine_3d.z_buffer[line][x_us] = pix_z;
                }

                // ===== alpha blend =====
                if self.engine_3d.disp3dcnt.alpha_blending && poly.translucent {
                    if self.engine_3d.trans_poly_ids[x_us] as i32 == poly.attributes.id {
                        continue;
                    }

                    #[cfg(feature = "tracing")]
                    tracing::trace!("Alpha: {alpha:02X}");

                    // Don't draw translucent polygons over each other if they share the same ID
                    self.engine_3d.trans_poly_ids[x_us] = poly.attributes.id as u8;

                    let engine = if is_engine_a {
                        &mut self.engine_upper
                    } else {
                        &mut self.engine_lower
                    };
                    let framebuffer = &mut engine.framebuffer;
                    let old = framebuffer[x_us + y_coord];

                    let pr = (old >> 16) & 0xFF;
                    let pg = (old >> 8) & 0xFF;
                    let pb = old & 0xFF;

                    r = (((alpha + 1) * r) + (31 - alpha) * pr) / 32;
                    g = (((alpha + 1) * g) + (31 - alpha) * pg) / 32;
                    b = (((alpha + 1) * b) + (31 - alpha) * pb) / 32;
                }

                let final_color = 0xFF000000 | ((r & 0xFF) << 16) | ((g & 0xFF) << 8) | (b & 0xFF);

                // ===== framebuffer select =====
                let engine = if is_engine_a {
                    &mut self.engine_upper
                } else {
                    &mut self.engine_lower
                };
                engine.framebuffer[x_us + y_coord] = final_color;
                engine.final_bg_priority[x_us] = bg0_priority;
            }
        }
    }
}
