use crate::common::{Gpu, read_palette_value};
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
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
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
        const SPRITE_SIZES: [[u32; 8]; 3] = [
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

                let priority = ((attributes[2] >> 10) & 0x3) as u8;
                if priority != bg_priority {
                    continue;
                }

                let rotscale = (attributes[0] & (1 << 8)) != 0;
                if rotscale {
                    self.draw_rotscale_sprite(&attributes, is_engine_a);
                    continue;
                }

                // Skip double-size sprites in normal mode
                if attributes[0] & (1 << 9) != 0 {
                    continue;
                }

                // Sprite position
                let x = (attributes[1] & 0x1FF) as u32;
                let mut y = (attributes[0] & 0xFF) as u32;
                y = (self.get_vcount() as u32 - y) & 0xFF;

                let shape = ((attributes[0] >> 14) & 0x3) as usize;
                let size = ((attributes[1] >> 14) & 0x3) as usize;
                let mode = ((attributes[0] >> 10) & 0x3) as usize;

                // Check vertical bounds
                if y >= SPRITE_SIZES[shape][size * 2 + 1] * 8 {
                    continue;
                }

                if mode == 3 {
                    // Bitmap OBJ mode (special handling)
                    let alpha = attributes[2] >> 12;
                    if alpha == 0 {
                        continue;
                    }
                    // alpha += 1;

                    let width = SPRITE_SIZES[shape][size * 2] * 8;
                    let tile_num = (attributes[2] & 0x3FF) as u32;

                    let tile_addr: u32;
                    {
                        let engine = match is_engine_a {
                            true => &mut self.engine_upper,
                            false => &mut self.engine_lower,
                        };

                        tile_addr = if engine.dispcnt.bitmap_obj_1d {
                            tile_num * (128 << engine.dispcnt.bitmap_obj_1d_bound as u32)
                                + (y * width * 2)
                        } else if engine.dispcnt.bitmap_obj_square {
                            ((tile_num & 0x1F) * 0x10) + ((tile_num & 0x3E0) * 0x80) + (y * 256 * 2)
                        } else {
                            ((tile_num & 0xF) * 0x10) + ((tile_num & 0x3F0) * 0x80) + (y * 128 * 2)
                        };
                    }

                    let mut pixel_addr = vram_obj_base + tile_addr;
                    for x_offset in x..(x + width) {
                        if x_offset >= PIXELS_PER_LINE as u32 {
                            pixel_addr += 2;
                            continue;
                        }

                        {
                            let engine = match is_engine_a {
                                true => &mut self.engine_upper,
                                false => &mut self.engine_lower,
                            };
                            if bg_priority > engine.final_bg_priority[x_offset as usize] {
                                pixel_addr += 2;
                                continue;
                            }
                        }

                        let color = match is_engine_a {
                            true => self.read_obja_u16(pixel_addr),
                            false => self.read_objb_u16(pixel_addr),
                        };
                        pixel_addr += 2;

                        if (color & (1 << 15)) != 0 {
                            let engine = match is_engine_a {
                                true => &mut self.engine_upper,
                                false => &mut self.engine_lower,
                            };
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

                    let tile_num = (attributes[2] & 0x3FF) as u32;
                    let palette = (attributes[2] >> 12) as u32;
                    let one_palette_mode = attributes[0] & (1 << 13) != 0;

                    let tile_y = y / 8;
                    let tile_scanline = y % 8 * if one_palette_mode { 8 } else { 4 };
                    let x_flip = attributes[1] & (1 << 12) != 0;

                    let y_dimension_num;

                    {
                        let engine = match is_engine_a {
                            true => &mut self.engine_upper,
                            false => &mut self.engine_lower,
                        };

                        // Determine y-dimension count
                        y_dimension_num = if engine.dispcnt.tile_obj_1d {
                            let mut y_dim = x_tiles << one_palette_mode as usize;
                            y_dim <<= engine.dispcnt.tile_obj_1d_bound;
                            y_dim
                        } else {
                            0x20
                        };
                    }

                    for tile in 0..x_tiles {
                        let mut tile_id = tile_y * y_dimension_num;
                        let tile_id_tmp = match x_flip {
                            true => x_tiles - tile - 1,
                            false => tile,
                        };
                        tile_id += tile_id_tmp << one_palette_mode as u32;
                        tile_id += tile_num;
                        tile_id <<= 5;

                        let tile_data = vram_obj_base + tile_id + tile_scanline;

                        if !one_palette_mode {
                            // 4bpp
                            let data = match is_engine_a {
                                true => self.read_obja_u32(tile_data),
                                false => self.read_objb_u32(tile_data),
                            };

                            let mut index = x + (tile * 8);
                            for i in 0..8 {
                                let color_index = if x_flip {
                                    (data >> ((7 - i) * 4)) & 0xF
                                } else {
                                    (data >> (i * 4)) & 0xF
                                };
                                let idx = (index & 0x1FF) as usize;
                                index += 1;

                                if idx >= PIXELS_PER_LINE || color_index == 0 {
                                    continue;
                                }

                                {
                                    let engine = match is_engine_a {
                                        true => &mut self.engine_upper,
                                        false => &mut self.engine_lower,
                                    };

                                    if priority > engine.final_bg_priority[idx as usize] {
                                        continue;
                                    }

                                    let address = 0x200 + palette as u32 * 32 + color_index * 2;

                                    engine.sprite_scanline[idx] = match is_engine_a {
                                        true => read_palette_value(&self.palette_upper, address),
                                        false => read_palette_value(&self.palette_lower, address),
                                    }
                                        as u32
                                        | (1 << 31);
                                }
                            }
                        } else {
                            // 8bpp
                            let data = match is_engine_a {
                                true => self.read_obja_u64(tile_data),
                                false => self.read_objb_u64(tile_data),
                            };

                            let mut index = x + (tile * 8);
                            for i in 0..8 {
                                let color_index = if x_flip {
                                    (data >> ((7 - i) * 8)) & 0xFF
                                } else {
                                    (data >> (i * 8)) & 0xFF
                                } as u32;
                                let idx = (index & 0x1FF) as u32;
                                index += 1;

                                if idx >= PIXELS_PER_LINE as u32 || color_index == 0 {
                                    continue;
                                }

                                {
                                    let (final_bg_priority, obj_extended_palette) = {
                                        let engine = match is_engine_a {
                                            true => &mut self.engine_upper,
                                            false => &mut self.engine_lower,
                                        };
                                        (
                                            engine.final_bg_priority[idx as usize],
                                            engine.dispcnt.obj_extended_palette,
                                        )
                                    };

                                    if priority > final_bg_priority {
                                        continue;
                                    }

                                    let obj_address = palette * 512 + color_index * 2;
                                    let address = 0x200 + color_index * 2;

                                    let sprite_scanline = match is_engine_a {
                                        true => match obj_extended_palette {
                                            true => self.read_extpal_obja(obj_address),
                                            false => {
                                                read_palette_value(&self.palette_upper, address)
                                            }
                                        },
                                        false => match obj_extended_palette {
                                            true => self.read_extpal_objb(obj_address),
                                            false => {
                                                read_palette_value(&self.palette_lower, address)
                                            }
                                        },
                                    }
                                        as u32
                                        | (1 << 31);

                                    {
                                        let engine = match is_engine_a {
                                            true => &mut self.engine_upper,
                                            false => &mut self.engine_lower,
                                        };
                                        engine.sprite_scanline[idx as usize] = sprite_scanline;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Copy sprite scanline to framebuffer with RGBA conversion
        let vcount = self.get_vcount();
        for i in 0..PIXELS_PER_LINE {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };

            if (engine.sprite_scanline[i] & (1 << 31)) == 0 {
                continue;
            }
            let color = engine.sprite_scanline[i];
            let r = ((color & 0x1F) << 3) as u32;
            let g = (((color >> 5) & 0x1F) << 3) as u32;
            let b = (((color >> 10) & 0x1F) << 3) as u32;
            engine.framebuffer[i + vcount as usize * PIXELS_PER_LINE] =
                0xFF000000 | (r << 16) | (g << 8) | b;
        }
    }

    /// Draws a single rotation/scaling sprite.
    ///
    /// `attributes` points to OAM attribute data.
    pub fn draw_rotscale_sprite(&mut self, attributes: &[u16; 4], is_engine_a: bool) {
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
        let mut width = (x_tiles * 8) as u32;
        let mut height = (y_tiles * 8) as u32;
        let mut x_bound = width;
        let mut y_bound = height;

        // Double-size flag
        if attributes[0] & (1 << 9) != 0 {
            x_bound *= 2;
            y_bound *= 2;
        }

        // Clip Y based on current scanline
        y = (self.get_vcount() as i32 - y) & 0xFF;
        if y as u32 >= y_bound {
            return;
        }

        // Sign-extend X
        x = (x << 23) >> 23;
        if x <= -(x_bound as i32) {
            return;
        }

        let center_x = x_bound / 2;
        let center_y = y_bound / 2;

        // Determine starting X offset and adjust bounds for clipping
        let x_offset: i32;
        if x >= 0 {
            x_offset = 0;
            if x + x_bound as i32 > 256 {
                x_bound = (256 - x) as u32;
            }
        } else {
            x_offset = -x;
            x = 0;
        }

        // Determine rotation/scaling matrix from OAM
        let rot_group = ((attributes[1] >> 9) & 0x1F) * 32;
        let oam_base = match is_engine_a {
            true => 0,
            false => 1024,
        };

        let base_address = (oam_base + rot_group) as u32;
        let rot_a = self.read_oam_i16(base_address + 0x6);
        let rot_b = self.read_oam_i16(base_address + 0xE);
        let rot_c = self.read_oam_i16(base_address + 0x16);
        let rot_d = self.read_oam_i16(base_address + 0x1E);

        // Compute starting rotated coordinates
        let mut rot_x = ((x_offset - center_x as i32) * rot_a as i32)
            + ((y - center_y as i32) * rot_b as i32)
            + (width << 7) as i32;
        let mut rot_y = ((x_offset - center_x as i32) * rot_c as i32)
            + ((y - center_y as i32) * rot_d as i32)
            + (height << 7) as i32;

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
        let y_dimension_num;
        {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };
            y_dimension_num = if engine.dispcnt.tile_obj_1d {
                tile_num <<= engine.dispcnt.tile_obj_1d_bound;
                (width >> 11) << one_palette_mode as usize
            } else {
                0x20
            } << 5;
        }

        tile_num <<= 5;

        // Compute pixel data base address
        let pixel_base = tile_num
            + match is_engine_a {
                true => VRAM_OBJA_START,
                false => VRAM_OBJB_START,
            };

        // Draw sprite pixels along scanline
        if one_palette_mode {
            let mut px_offset = x_offset;
            let mut px = x;
            while px_offset < x_bound as i32 {
                if rot_x as u32 <= width && rot_y as u32 <= height {
                    let mut pixel_addr = pixel_base;
                    pixel_addr += ((rot_y >> 11) * y_dimension_num as i32) as u32;
                    pixel_addr += ((rot_y & 0x700) >> 5) as u32;
                    pixel_addr += ((rot_x >> 11) * 64) as u32;
                    pixel_addr += ((rot_x & 0x700) >> 8) as u32;

                    let mut color = match is_engine_a {
                        true => self.read_obja_u8(pixel_addr),
                        false => self.read_objb_u8(pixel_addr),
                    } as u16;

                    let (is_priority, obj_extended_palette) = {
                        let engine = match is_engine_a {
                            true => &mut self.engine_upper,
                            false => &mut self.engine_lower,
                        };
                        (
                            priority <= engine.final_bg_priority[px as usize],
                            engine.dispcnt.obj_extended_palette,
                        )
                    };

                    if color != 0 && is_priority {
                        let palette_address = 0x200 + color as u32 * 2;
                        color = if obj_extended_palette {
                            let obj_addr = palette_id * 512 + color as u32 * 2;
                            if is_engine_a {
                                self.read_extpal_obja(obj_addr)
                            } else {
                                self.read_extpal_objb(obj_addr)
                            }
                        } else if is_engine_a {
                            read_palette_value(&self.palette_upper, palette_address)
                        } else {
                            read_palette_value(&self.palette_lower, palette_address)
                        };
                        {
                            let engine = match is_engine_a {
                                true => &mut self.engine_upper,
                                false => &mut self.engine_lower,
                            };
                            engine.sprite_scanline[px as usize] = color as u32 | (1 << 31);
                        }
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
            while px_offset < x_bound as i32 {
                if rot_x as u32 <= width as u32 && rot_y as u32 <= height as u32 {
                    let mut pixel_addr = pixel_base;
                    pixel_addr += ((rot_y >> 11) * y_dimension_num as i32) as u32;
                    pixel_addr += ((rot_y & 0x700) >> 6) as u32;
                    pixel_addr += ((rot_x >> 11) * 32) as u32;
                    pixel_addr += ((rot_x & 0x700) >> 9) as u32;

                    let mut color = match is_engine_a {
                        true => self.read_obja_u8(pixel_addr),
                        false => self.read_objb_u8(pixel_addr),
                    } as u16;

                    if rot_x & 0x100 != 0 {
                        color >>= 4;
                    } else {
                        color &= 0xF;
                    }

                    let is_priority = {
                        let engine = match is_engine_a {
                            true => &mut self.engine_upper,
                            false => &mut self.engine_lower,
                        };
                        priority <= engine.final_bg_priority[px as usize]
                    };

                    if color != 0 && is_priority {
                        let palette_addr = 0x200 + palette_id * 32 + color as u32 * 2;

                        color = if is_engine_a {
                            read_palette_value(&self.palette_upper, palette_addr)
                        } else {
                            read_palette_value(&self.palette_lower, palette_addr)
                        };
                        {
                            let engine = match is_engine_a {
                                true => &mut self.engine_upper,
                                false => &mut self.engine_lower,
                            };
                            engine.sprite_scanline[px as usize] = color as u32 | (1 << 31);
                        }
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
