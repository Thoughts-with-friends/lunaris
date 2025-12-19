use crate::common::Gpu;
use mem_const::*;

impl Gpu {
    /// Draws all sprites for the current scanline.
    pub fn draw_sprites(&mut self, is_engine_a: bool) {
        // Temporary buffers for this scanline
        let mut attributes = [0u16; 4];
        let mut colors = [0u16; PIXELS_PER_LINE * 2];

        {
            // Clear buffers
            let engine = match is_engine_a {
                true => &mut self.eng_a,
                false => &mut self.eng_b,
            };
            engine.sprite_scanline.fill(0);
            colors.fill(0);
        }

        // Determine OAM and VRAM base for this engine
        let (oam_base, vram_obj_base) = match is_engine_a {
            true => (0, VRAM_OBJA_START),
            false => (1024, VRAM_OBJB_START),
        };

        // Sprite size tables: [shape][size index]
        const SPRITE_SIZES: [[usize; 8]; 3] = [
            [1, 1, 2, 2, 4, 4, 8, 8], // Square
            [2, 1, 4, 1, 4, 2, 8, 4], // Horizontal
            [1, 2, 1, 4, 2, 4, 4, 8], // Vertical
        ];

        // Iterate over BG priorities (highest first)
        for bg_priority in (0..=3).rev() {
            // Iterate over 128 sprites (highest index first)
            for i in (0..128).rev() {
                // Read sprite attributes from OAM
                attributes[0] = self.read_oam_u16(oam_base + i * 8);
                attributes[1] = self.read_oam_u16(oam_base + i * 8 + 2);
                attributes[2] = self.read_oam_u16(oam_base + i * 8 + 4);
                attributes[3] = self.read_oam_u16(oam_base + i * 8 + 6);

                let priority = ((attributes[2] >> 10) & 0x3) as usize;
                if priority != bg_priority {
                    continue;
                }

                let rotscale = (attributes[0] & (1 << 8)) != 0;
                if rotscale {
                    self.draw_rotscale_sprite(&attributes);
                    continue;
                }

                // Skip double-size sprites in normal mode
                if attributes[0] & (1 << 9) != 0 {
                    continue;
                }

                // Sprite position
                let mut x = (attributes[1] & 0x1FF) as i32;
                let mut y = (attributes[0] & 0xFF) as i32;
                y = (self.get_vcount() as i32 - y) & 0xFF;

                let shape = ((attributes[0] >> 14) & 0x3) as usize;
                let size = ((attributes[1] >> 14) & 0x3) as usize;
                let mode = ((attributes[0] >> 10) & 0x3) as usize;

                // Check vertical bounds
                if y as usize >= SPRITE_SIZES[shape][size * 2 + 1] * 8 {
                    continue;
                }

                if mode == 3 {
                    // Bitmap OBJ mode (special handling)
                    let alpha = attributes[2] >> 12;
                    if alpha == 0 {
                        continue;
                    }

                    let alpha = alpha + 1;
                    let width = SPRITE_SIZES[shape][size * 2] * 8;
                    let tile_num = (attributes[2] & 0x3FF) as u32;

                    let mut tile_addr: u32;
                    {
                        let engine = match is_engine_a {
                            true => &mut self.eng_a,
                            false => &mut self.eng_b,
                        };

                        tile_addr = if engine.dispcnt.bitmap_obj_1d {
                            tile_num * (128 << engine.dispcnt.bitmap_obj_1d_bound as u32)
                                + (y as u32 * width as u32 * 2)
                        } else if engine.dispcnt.bitmap_obj_square {
                            ((tile_num & 0x1F) * 0x10)
                                + ((tile_num & 0x3E0) * 0x80)
                                + (y as u32 * 256 * 2)
                        } else {
                            ((tile_num & 0xF) * 0x10)
                                + ((tile_num & 0x3F0) * 0x80)
                                + (y as u32 * 128 * 2)
                        };
                    }

                    let mut pixel_addr = vram_obj_base + tile_addr;
                    for x_offset in x..(x + width as i32) {
                        if x_offset < 0 || x_offset >= PIXELS_PER_LINE as i32 {
                            pixel_addr += 2;
                            continue;
                        }

                        let engine = match is_engine_a {
                            true => &mut self.eng_a,
                            false => &mut self.eng_b,
                        };
                        if bg_priority > engine.final_bg_priority[x_offset as usize] as usize {
                            pixel_addr += 2;
                            continue;
                        }

                        let color = match is_engine_a {
                            true => self.read_obja_u16(pixel_addr),
                            false => self.read_objb_u16(pixel_addr),
                        };
                        pixel_addr += 2;

                        if (color & (1 << 15)) != 0 {
                            engine.sprite_scanline[x_offset as usize] = color as u32 | (1 << 31);
                        }
                    }
                } else {
                    // Normal palette-based OBJ
                    let x_tiles = SPRITE_SIZES[shape][size * 2];
                    let y_tiles = SPRITE_SIZES[shape][size * 2 + 1];

                    if attributes[1] & 0x2000 != 0 {
                        y = ((y_tiles * 8) - 1) - y;
                    }

                    let tile_num = (attributes[2] & 0x3FF) as usize;
                    let palette = (attributes[2] >> 12) as usize;
                    let one_palette_mode = attributes[0] & (1 << 13) != 0;

                    let tile_y = y / 8;
                    let tile_scanline = y % 8 * if one_palette_mode { 8 } else { 4 };
                    let x_flip = attributes[1] & (1 << 12) != 0;

                    // Determine y-dimension count
                    let y_dimension_num = if self.dispcnt.tile_obj_1d {
                        let mut y_dim = x_tiles << one_palette_mode as usize;
                        y_dim <<= self.dispcnt.tile_obj_1d_bound;
                        y_dim
                    } else {
                        0x20
                    };

                    for tile in 0..x_tiles {
                        let mut tile_id = tile_y * y_dimension_num;
                        tile_id +=
                            ((if x_flip { x_tiles - tile - 1 } else { tile }) << one_palette_mode);
                        tile_id += tile_num;
                        tile_id <<= 5;

                        let tile_data = vram_obj_base + tile_id + tile_scanline as usize;

                        if !one_palette_mode {
                            // 4bpp
                            let data = if self.engine_a {
                                self.read_obja::<u32>(tile_data)
                            } else {
                                self.read_objb::<u32>(tile_data)
                            };

                            let mut index = x + (tile as i32 * 8);
                            for i in 0..8 {
                                let color_index = if x_flip {
                                    ((data >> ((7 - i) * 4)) & 0xF) as usize
                                } else {
                                    ((data >> (i * 4)) & 0xF) as usize
                                };
                                let idx = (index & 0x1FF) as usize;
                                index += 1;

                                if idx >= PIXELS_PER_LINE || color_index == 0 {
                                    continue;
                                }
                                if priority > self.final_bg_priority[idx] {
                                    continue;
                                }

                                self.sprite_scanline[idx] = if self.engine_a {
                                    self.gpu
                                        .read_palette_a(0x200 + palette * 32 + color_index * 2)
                                } else {
                                    self.gpu
                                        .read_palette_b(0x200 + palette * 32 + color_index * 2)
                                } | (1 << 31);
                            }
                        } else {
                            // 8bpp
                            let data = if self.engine_a {
                                self.read_obja::<u64>(tile_data)
                            } else {
                                self.read_objb::<u64>(tile_data)
                            };

                            let mut index = x + (tile as i32 * 8);
                            for i in 0..8 {
                                let color_index = if x_flip {
                                    ((data >> ((7 - i) * 8)) & 0xFF) as usize
                                } else {
                                    ((data >> (i * 8)) & 0xFF) as usize
                                };
                                let idx = (index & 0x1FF) as usize;
                                index += 1;

                                if idx >= PIXELS_PER_LINE || color_index == 0 {
                                    continue;
                                }
                                if priority > self.final_bg_priority[idx] {
                                    continue;
                                }

                                self.sprite_scanline[idx] = if self.engine_a {
                                    if self.dispcnt.obj_extended_palette {
                                        self.read_extpal_obja(palette * 512 + color_index * 2)
                                    } else {
                                        self.read_palette_a(0x200 + color_index * 2)
                                    }
                                } else {
                                    if self.dispcnt.obj_extended_palette {
                                        self.read_extpal_objb(palette * 512 + color_index * 2)
                                    } else {
                                        self.read_palette_b(0x200 + color_index * 2)
                                    }
                                } | (1 << 31);
                            }
                        }
                    }
                }
            }
        }

        // Copy sprite scanline to framebuffer with RGBA conversion
        let vcount = self.get_vcount();
        for i in 0..PIXELS_PER_LINE {
            if (self.sprite_scanline[i] & (1 << 31)) == 0 {
                continue;
            }
            let color = self.sprite_scanline[i];
            let r = ((color & 0x1F) << 3) as u32;
            let g = (((color >> 5) & 0x1F) << 3) as u32;
            let b = (((color >> 10) & 0x1F) << 3) as u32;
            self.framebuffer[i + vcount as usize * PIXELS_PER_LINE] =
                0xFF000000 | (r << 16) | (g << 8) | b;
        }
    }

    /// Draws a single rotation/scaling sprite.
    ///
    /// `attributes` points to OAM attribute data.
    pub fn draw_rotscale_sprite(&mut self, attributes: &[u16; 4]) {
        // Sprite size tables: [shape][size index]
        const SPRITE_SIZES: [[usize; 8]; 3] = [
            [1, 1, 2, 2, 4, 4, 8, 8], // Square
            [2, 1, 4, 1, 4, 2, 8, 4], // Horizontal
            [1, 2, 1, 4, 2, 4, 4, 8], // Vertical
        ];

        // Read position and shape/size/priority
        let mut x = (attributes[1] & 0x1FF) as i32;
        let mut y = (attributes[0] & 0xFF) as i32;
        let shape = ((attributes[0] >> 14) & 0x3) as usize;
        let size = ((attributes[1] >> 14) & 0x3) as usize;
        let priority = ((attributes[2] >> 10) & 0x3) as u8;

        // Determine number of tiles horizontally and vertically
        let x_tiles = SPRITE_SIZES[shape][size * 2];
        let y_tiles = SPRITE_SIZES[shape][size * 2 + 1];

        // Compute width and height in pixels
        let mut width = x_tiles * 8;
        let mut height = y_tiles * 8;
        let mut x_bound = width;
        let mut y_bound = height;

        // Double-size flag
        if attributes[0] & (1 << 9) != 0 {
            x_bound *= 2;
            y_bound *= 2;
        }

        // Clip Y based on current scanline
        y = (self.get_vcount() as i32 - y) & 0xFF;
        if y as u32 >= y_bound as u32 {
            return;
        }

        // Sign-extend X
        x = (x << 23) >> 23;
        if x <= -x_bound as i32 {
            return;
        }

        let center_x = x_bound as i32 / 2;
        let center_y = y_bound as i32 / 2;

        // Determine starting X offset and adjust bounds for clipping
        let mut x_offset: i32;
        if x >= 0 {
            x_offset = 0;
            if x + x_bound > 256 {
                x_bound = 256 - x;
            }
        } else {
            x_offset = -x;
            x = 0;
        }

        // Determine rotation/scaling matrix from OAM
        let rot_group = ((attributes[1] >> 9) & 0x1F) * 32;
        let oam_base = if self.engine_a { 0 } else { 1024 };

        let rot_a = self.read_oam::<i16>(oam_base + rot_group + 0x6);
        let rot_b = self.read_oam::<i16>(oam_base + rot_group + 0xE);
        let rot_c = self.read_oam::<i16>(oam_base + rot_group + 0x16);
        let rot_d = self.read_oam::<i16>(oam_base + rot_group + 0x1E);

        // Compute starting rotated coordinates
        let mut rot_x =
            ((x_offset - center_x) * rot_a as i32) + ((y - center_y) * rot_b as i32) + (width << 7);
        let mut rot_y = ((x_offset - center_x) * rot_c as i32)
            + ((y - center_y) * rot_d as i32)
            + (height << 7);

        width <<= 8;
        height <<= 8;

        // Tile index and palette information
        let mut tile_num = (attributes[2] & 0x3FF) as u32;
        let palette_id = (attributes[2] >> 12) as u32;
        let one_palette_mode = attributes[0] & 0x2000 != 0;

        // Skip invalid shape
        if shape == 3 {
            return;
        }

        // Determine tile addressing (1D/2D)
        let y_dimension_num = if self.dispcnt.tile_obj_1d {
            tile_num <<= self.dispcnt.tile_obj_1d_bound;
            ((width >> 11) << one_palette_mode as usize)
        } else {
            0x20
        } << 5;
        tile_num <<= 5;

        // Compute pixel data base address
        let pixel_base = tile_num
            + if self.engine_a {
                VRAM_OBJA_START
            } else {
                VRAM_OBJB_START
            };

        // Draw sprite pixels along scanline
        if one_palette_mode {
            let mut px_offset = x_offset;
            let mut px = x;
            while px_offset < x_bound {
                if rot_x as u32 <= width as u32 && rot_y as u32 <= height as u32 {
                    let mut pixel_addr = pixel_base;
                    pixel_addr += ((rot_y >> 11) * y_dimension_num as i32) as u32;
                    pixel_addr += ((rot_y & 0x700) >> 5) as u32;
                    pixel_addr += ((rot_x >> 11) * 64) as u32;
                    pixel_addr += ((rot_x & 0x700) >> 8) as u32;

                    let mut color = if self.engine_a {
                        self.read_obja::<u8>(pixel_addr)
                    } else {
                        self.read_objb::<u8>(pixel_addr)
                    };

                    if color != 0 && priority <= self.final_bg_priority[px as usize] {
                        color = if self.dispcnt.obj_extended_palette {
                            if self.engine_a {
                                self.gpu
                                    .read_extpal_obja(palette_id * 512 + color as u32 * 2)
                            } else {
                                self.gpu
                                    .read_extpal_objb(palette_id * 512 + color as u32 * 2)
                            }
                        } else if self.engine_a {
                            self.read_palette_a(0x200 + color as u32 * 2)
                        } else {
                            self.read_palette_b(0x200 + color as u32 * 2)
                        };

                        self.sprite_scanline[px as usize] = color | (1 << 31);
                    }
                }
                rot_x += rot_a as i32;
                rot_y += rot_c as i32;
                px_offset += 1;
                px += 1;
            }
        } else {
            // Two-palette mode
            let mut px_offset = x_offset;
            let mut px = x;
            while px_offset < x_bound {
                if rot_x as u32 <= width as u32 && rot_y as u32 <= height as u32 {
                    let mut pixel_addr = pixel_base;
                    pixel_addr += ((rot_y >> 11) * y_dimension_num as i32) as u32;
                    pixel_addr += ((rot_y & 0x700) >> 6) as u32;
                    pixel_addr += ((rot_x >> 11) * 32) as u32;
                    pixel_addr += ((rot_x & 0x700) >> 9) as u32;

                    let mut color = if self.engine_a {
                        self.read_obja::<u8>(pixel_addr)
                    } else {
                        self.read_objb::<u8>(pixel_addr)
                    };

                    if rot_x & 0x100 != 0 {
                        color >>= 4;
                    } else {
                        color &= 0xF;
                    }

                    if color != 0 && priority <= self.final_bg_priority[px as usize] {
                        let palette_addr = 0x200 + palette_id * 32 + color as u32 * 2;
                        self.sprite_scanline[px as usize] = if self.engine_a {
                            self.read_palette_a(palette_addr)
                        } else {
                            self.read_palette_b(palette_addr)
                        } | (1 << 31);
                    }
                }
                rot_x += rot_a as i32;
                rot_y += rot_c as i32;
                px_offset += 1;
                px += 1;
            }
        }
    }
}
