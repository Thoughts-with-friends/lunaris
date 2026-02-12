// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpueng.cpp
//!
//! See: https://github.com/PSI-Rockin/CorgiDS/blob/0040fccf587ae04c041db562ced4c07f3d937594/src/gpueng.cpp#L363
//!
//! CorgiDS was calling GPU methods in Engine2D, but this caused a circular reference.
//! To avoid this, we've implemented the method in the parent here.
use crate::gpu_root::{Gpu, bytes_to_palette, read_palette_value};
use lunaris_ds_mem_const::*;

impl Gpu {
    // ============================================================
    // Rendering pipeline
    // ============================================================
    /// Compute window mask for the current scanline.
    pub fn get_window_mask(&mut self, is_engine_a: bool) {
        // Determine if the windows are active on this scanline
        // Note: only the lower 8 bits of VCOUNT are used
        let line = (self.get_vcount() & 0xFF) as i32;

        let engine = match is_engine_a {
            true => &mut self.engine_upper,
            false => &mut self.engine_lower,
        };

        let y1_0 = (engine.win0v >> 8) as i32;
        let y2_0 = (engine.win0v & 0xFF) as i32;
        let y1_1 = (engine.win1v >> 8) as i32;
        let y2_1 = (engine.win1v & 0xFF) as i32;

        if line == y1_0 {
            engine.win0_active = true;
        } else if line == y2_0 {
            engine.win0_active = false;
        }

        if line == y1_1 {
            engine.win1_active = true;
        } else if line == y2_1 {
            engine.win1_active = false;
        }

        // Reset window mask to outside window
        let outside = (engine.get_winout() & 0xFF) as u8;
        for i in 0..PIXELS_PER_LINE {
            engine.window_mask[i] = outside;
        }

        if engine.dispcnt.obj_win_display {
            // TODO: WINOBJ
        }

        if engine.dispcnt.display_win1 && engine.win1_active {
            let x1 = (engine.win1h >> 8) as usize;
            let x2 = (engine.win1h & 0xFF) as usize;
            let mask = (engine.get_winin() >> 8) as u8;

            for x in x1..x2 {
                if x < PIXELS_PER_LINE {
                    engine.window_mask[x] = mask;
                }
            }
        }

        if engine.dispcnt.display_win0 && engine.win0_active {
            let x1 = (engine.win0h >> 8) as usize;
            let x2 = (engine.win0h & 0xFF) as usize;
            let mask = (engine.get_winin() & 0xFF) as u8;

            for x in x1..x2 {
                if x < PIXELS_PER_LINE {
                    engine.window_mask[x] = mask;
                }
            }
        }
    }

    pub fn handle_bldcnt_effects(&mut self) {
        // Nothing impl in CorgiDS.
        todo!()
    }

    /// Draws an extended text background (BG2 or BG3) for the current scanline.
    pub fn draw_ext_text(&mut self, index: usize, is_engine_a: bool) {
        let (rot_a, rot_b, rot_c, rot_d): (i16, i16, i16, i16);
        let (mut x_offset, mut y_offset): (i32, i32);
        let overflow_mask: u32;
        let mask: u32;
        let mut y_factor: u32;
        let mut screen_base: u32;
        let mut char_base: u32;
        let extpal_base: u32;

        {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };

            // Load rotation/scaling matrix and offsets
            (rot_a, rot_b, rot_c, rot_d) = match index {
                2 => (
                    engine.bg2p_internal[0] as i16,
                    engine.bg2p_internal[1] as i16,
                    engine.bg2p_internal[2] as i16,
                    engine.bg2p_internal[3] as i16,
                ),
                _ => (
                    engine.bg3p_internal[0] as i16,
                    engine.bg3p_internal[1] as i16,
                    engine.bg3p_internal[2] as i16,
                    engine.bg3p_internal[3] as i16,
                ),
            };

            (x_offset, y_offset) = match index {
                2 => (engine.bg2x_internal, engine.bg2y_internal),
                _ => (engine.bg3x_internal, engine.bg3y_internal),
            };

            // Determine base addresses for screen and character data
            (screen_base, char_base) = match is_engine_a {
                true => (
                    VRAM_BGA_START + (engine.dispcnt.screen_base as u32 * 1024 * 64),
                    VRAM_BGA_START + (engine.dispcnt.char_base as u32 * 1024 * 64),
                ),
                false => (VRAM_BGB_C, VRAM_BGB_C),
            };

            // Apply BG-specific offsets
            screen_base += (((engine.bgcnt[index] >> 8) & 0x1F) as u32) * 1024 * 2;
            char_base += (((engine.bgcnt[index] >> 2) & 0xF) as u32) * 1024 * 16;

            // Determine screen size and mask
            let screen_size = (engine.bgcnt[index] >> 14) & 0x3;
            mask = match screen_size {
                0 => 0x7800,
                1 => 0xF800,
                2 => 0x1F800,
                3 => 0x3F800,
                _ => 0x7800, // default fallback
            };
            y_factor = screen_size as u32 + 7;
            y_factor -= 3;

            extpal_base = index as u32 * 1024 * 8;
            overflow_mask = if (engine.bgcnt[index] & (1 << 13)) != 0 {
                0
            } else {
                !(mask | 0x7FF)
            };
        }

        // Iterate over pixels in the current scanline
        for pixel in 0..PIXELS_PER_LINE {
            if ((x_offset | y_offset) & overflow_mask as i32) == 0 {
                // Compute tile offset
                let mut tile_addr_offset = ((y_offset as u32 & mask) >> 11) << y_factor;
                tile_addr_offset += (x_offset as u32 & mask) >> 11;
                tile_addr_offset <<= 1;

                // Read tile ID
                let tile: u16 = match is_engine_a {
                    true => self.read_bga_u16(screen_base + tile_addr_offset),
                    false => self.read_bgb_u16(screen_base + tile_addr_offset),
                };

                let char_id = (tile & 0x3FF) as u32;
                let x_flip = (tile & (1 << 10)) != 0;
                let y_flip = (tile & (1 << 11)) != 0;
                let palette_id = (tile >> 12) & 0xF;

                // Determine tile pixel coordinates
                let mut tile_x_offset = ((x_offset >> 8) & 0x7) as u32;
                let mut tile_y_offset = ((y_offset >> 8) & 0x7) as u32;
                if x_flip {
                    tile_x_offset = 7 - tile_x_offset;
                }
                if y_flip {
                    tile_y_offset = 7 - tile_y_offset;
                }

                // Read color index from character data
                let address = char_base + (char_id << 6) + (tile_y_offset << 3) + tile_x_offset;
                let mut color: u16 = match is_engine_a {
                    true => self.read_bga_u8(address) as u16,
                    false => self.read_bgb_u8(address) as u16,
                };

                if color != 0 {
                    let bg_extended_palette = {
                        let engine = match is_engine_a {
                            true => &mut self.engine_upper,
                            false => &mut self.engine_lower,
                        };
                        engine.dispcnt.bg_extended_palette
                    };

                    // Convert to true color
                    color = if bg_extended_palette {
                        let address = extpal_base + color as u32 * 2 + palette_id as u32 * 512;
                        match is_engine_a {
                            true => self.read_extpal_bga_u16(address),
                            false => self.read_extpal_bgb_u16(address),
                        }
                    } else {
                        match is_engine_a {
                            true => read_palette_value(&self.palette_upper, color as u32 * 2),
                            false => read_palette_value(&self.palette_lower, color as u32 * 2),
                        }
                    };

                    let r = ((color & 0x1F) << 3) as u32;
                    let g = (((color >> 5) & 0x1F) << 3) as u32;
                    let b = (((color >> 10) & 0x1F) << 3) as u32;
                    let true_color = 0xFF000000 | (r << 16) | (g << 8) | b;

                    let address = pixel + (self.get_vcount() as usize * PIXELS_PER_LINE);

                    let engine = match is_engine_a {
                        true => &mut self.engine_upper,
                        false => &mut self.engine_lower,
                    };
                    engine.framebuffer[address] = true_color;
                    engine.final_bg_priority[pixel] = (engine.bgcnt[index] & 0x3) as u8;
                }
            }

            // Advance coordinates using rotation/scaling matrix
            x_offset += rot_a as i32;
            y_offset += rot_c as i32;
        }

        let engine = match is_engine_a {
            true => &mut self.engine_upper,
            false => &mut self.engine_lower,
        };

        // Update BG internal offsets for next scanline
        if index == 2 {
            engine.bg2x_internal += rot_b as i32;
            engine.bg2y_internal += rot_d as i32;
        } else {
            engine.bg3x_internal += rot_b as i32;
            engine.bg3y_internal += rot_d as i32;
        }
    }

    /// Draws the backdrop (background color layer).
    pub fn draw_backdrop(&mut self, is_engine_a: bool) {
        let palette = bytes_to_palette(match is_engine_a {
            true => &self.palette_upper,
            false => &self.palette_lower,
        });
        let c = palette[0];

        let y = self.get_vcount() as usize;
        let base = y * PIXELS_PER_LINE;

        for x in 0..PIXELS_PER_LINE {
            let r = ((c & 0x1F) << 3) as u32;
            let g = (((c >> 5) & 0x1F) << 3) as u32;
            let b = (((c >> 10) & 0x1F) << 3) as u32;

            let color = 0xFF000000 | (r << 16) | (g << 8) | b;

            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };
            engine.framebuffer[base + x] = color;
        }
    }

    /// Draws a text background layer.
    ///
    /// `index` must be 0..=3.
    pub fn draw_bg_txt(&mut self, index: usize, is_engine_a: bool) {
        let palette = bytes_to_palette(match is_engine_a {
            true => &self.palette_upper,
            false => &self.palette_lower,
        });

        let v_count = self.get_vcount();

        let mut x_offset: u16;
        let y_offset: u16;
        let one_palette_mode: bool;
        let mut screen_base: u32;
        let mut char_base: u32;
        let wide_x: u16;
        let scanline = v_count as usize * PIXELS_PER_LINE;

        {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };

            x_offset = engine.bghofs[index];
            y_offset = engine.bgvofs[index].wrapping_add(v_count);

            one_palette_mode = (engine.bgcnt[index] & (1 << 7)) != 0;

            screen_base = match is_engine_a {
                true => VRAM_BGA_START + (engine.dispcnt.screen_base as u32 * 1024 * 64),
                false => VRAM_BGB_C,
            };
            char_base = match is_engine_a {
                true => VRAM_BGA_START + (engine.dispcnt.char_base as u32 * 1024 * 64),
                false => VRAM_BGB_C,
            };

            screen_base += ((engine.bgcnt[index] >> 8) & 0x1F) as u32 * 1024 * 2;

            if (engine.bgcnt[index] & 0x8000) != 0 {
                screen_base += ((y_offset & 0x1F8) as u32) * 8;
                if (engine.bgcnt[index] & 0x4000) != 0 {
                    screen_base += ((y_offset & 0x100) as u32) * 8;
                }
            } else {
                screen_base += ((y_offset & 0xF8) as u32) * 8;
            }

            char_base += ((engine.bgcnt[index] >> 2) & 0xF) as u32 * 1024 * 16;

            wide_x = match (engine.bgcnt[index] & (1 << 14)) != 0 {
                true => 0x100,
                false => 0,
            };
        }

        if !one_palette_mode {
            // 16-color tiles
            let mut data: u32 = 0;
            let mut palette_id = 0;
            let mut x_flip: bool = false;

            if (x_offset & 0x7) != 0 {
                let addr = screen_base
                    + (((x_offset & 0xF8) as u32) >> 2)
                    + (((x_offset & wide_x) as u32) << 3);

                let tile = match is_engine_a {
                    true => self.read_bga_u16(addr),
                    false => self.read_bgb_u16(addr),
                };

                let tile_num = tile & 0x3FF;
                x_flip = (tile & (1 << 10)) != 0;
                let y_flip = (tile & (1 << 11)) != 0;
                palette_id = tile >> 12;

                let pixel_base = char_base
                    + (tile_num as u32 * 32)
                    + if y_flip {
                        (7 - (y_offset & 0x7)) as u32 * 4
                    } else {
                        (y_offset & 0x7) as u32 * 4
                    };

                data = match is_engine_a {
                    true => self.read_bga_u32(addr),
                    false => self.read_bgb_u32(pixel_base),
                };
            }

            let addr = screen_base
                + (((x_offset & 0xF8) as u32) >> 2)
                + (((x_offset & wide_x) as u32) << 3);
            let tile = match is_engine_a {
                true => self.read_bga_u16(addr),
                false => self.read_bgb_u16(addr),
            };

            if (x_offset & 0x7) == 0 {
                let tile_num = tile & 0x3FF;
                x_flip = (tile & (1 << 10)) != 0;
                let y_flip = (tile & (1 << 11)) != 0;
                palette_id = tile >> 12;

                let pixel_base = char_base
                    + (tile_num as u32 * 32)
                    + if y_flip {
                        (7 - (y_offset & 0x7)) as u32 * 4
                    } else {
                        (y_offset & 0x7) as u32 * 4
                    };
                data = match is_engine_a {
                    true => self.read_bga_u32(addr),
                    false => self.read_bgb_u32(pixel_base),
                };
            }

            let tile_x = match x_flip {
                true => 7 - (x_offset & 0x7),
                false => x_offset & 0x7,
            };
            let color = ((data >> (tile_x * 4)) & 0xF) as u16;

            for pixel in 0..PIXELS_PER_LINE {
                let engine = match is_engine_a {
                    true => &mut self.engine_upper,
                    false => &mut self.engine_lower,
                };

                if color != 0 && (engine.window_mask[pixel] & (1 << index)) != 0 {
                    let pal_color = palette[(palette_id as usize * 16) + color as usize];

                    let r = ((pal_color & 0x1F) << 3) as u32;
                    let g = (((pal_color >> 5) & 0x1F) << 3) as u32;
                    let b = (((pal_color >> 10) & 0x1F) << 3) as u32;

                    engine.framebuffer[pixel + scanline] = 0xFF000000 | (r << 16) | (g << 8) | b;
                    engine.final_bg_priority[pixel] = (engine.bgcnt[index] & 0x3) as u8;
                }

                x_offset = x_offset.wrapping_add(1);
            }
        } else {
            // 256-color tiles
            let mut data: u64 = 0;
            let mut palette_id: u16 = 0;
            let mut x_flip: bool = false;

            if (x_offset & 0x7) != 0 {
                let addr = screen_base
                    + (((x_offset & 0xF8) as u32) >> 2)
                    + (((x_offset & wide_x) as u32) << 3);

                let tile = match is_engine_a {
                    true => self.read_bga_u16(addr),
                    false => self.read_bgb_u16(addr),
                };

                let tile_num = tile & 0x3FF;
                x_flip = (tile & (1 << 10)) != 0;
                let y_flip = (tile & (1 << 11)) != 0;
                palette_id = tile >> 12;

                let pixel_base = char_base
                    + (tile_num as u32 * 64)
                    + if y_flip {
                        (7 - (y_offset & 0x7)) as u32 * 8
                    } else {
                        (y_offset & 0x7) as u32 * 8
                    };

                data = match is_engine_a {
                    true => self.read_bga_u64(addr),
                    false => self.read_bgb_u64(pixel_base),
                };
            }

            for pixel in 0..PIXELS_PER_LINE {
                if (x_offset & 0x7) == 0 {
                    let addr = screen_base
                        + (((x_offset & 0xF8) as u32) >> 2)
                        + (((x_offset & wide_x) as u32) << 3);

                    let tile = self.read_bga_u16(addr);

                    let tile_num = tile & 0x3FF;
                    x_flip = (tile & (1 << 10)) != 0;
                    let y_flip = (tile & (1 << 11)) != 0;
                    palette_id = tile >> 12;

                    let pixel_base = char_base
                        + (tile_num as u32 * 64)
                        + if y_flip {
                            (7 - (y_offset & 0x7)) as u32 * 8
                        } else {
                            (y_offset & 0x7) as u32 * 8
                        };

                    data = match is_engine_a {
                        true => self.read_bga_u64(addr),
                        false => self.read_bgb_u64(pixel_base),
                    };
                }

                let tile_x = if x_flip {
                    7 - (x_offset & 0x7)
                } else {
                    x_offset & 0x7
                };

                let mut color = ((data >> (tile_x * 8)) & 0xFF) as u16;

                let (window_mask_byte, bg_extended_palette) = match is_engine_a {
                    true => (
                        &self.engine_upper.window_mask[pixel],
                        self.engine_upper.dispcnt.bg_extended_palette,
                    ),
                    false => (
                        &self.engine_lower.window_mask[pixel],
                        self.engine_upper.dispcnt.bg_extended_palette,
                    ),
                };

                if color != 0 && (window_mask_byte & (1 << index)) != 0 {
                    if bg_extended_palette {
                        let ext_base = index as u32 * 1024 * 8;
                        let address = ext_base + (palette_id as u32 * 512) + (color as u32 * 2);

                        color = match is_engine_a {
                            true => self.read_extpal_bga_u16(address),
                            false => self.read_extpal_bgb_u16(address),
                        };
                    } else {
                        color = palette[color as usize];
                    }

                    let r = ((color & 0x1F) << 3) as u32;
                    let g = (((color >> 5) & 0x1F) << 3) as u32;
                    let b = (((color >> 10) & 0x1F) << 3) as u32;

                    let engine = match is_engine_a {
                        true => &mut self.engine_upper,
                        false => &mut self.engine_lower,
                    };
                    engine.framebuffer[pixel + scanline] = 0xFF000000 | (r << 16) | (g << 8) | b;
                    engine.final_bg_priority[pixel] = (engine.bgcnt[index] & 0x3) as u8;
                }

                x_offset = x_offset.wrapping_add(1);
            }
        }
    }

    /// Draws an extended/affine background layer.
    ///
    /// `index` is typically 2 or 3.
    pub fn draw_bg_ext(&mut self, index: usize, is_engine_a: bool) {
        // Determine the base VRAM address depending on the engine
        let mut base: u32 = match is_engine_a {
            true => VRAM_BGA_START,
            false => VRAM_BGB_C,
        };

        // Get current vertical scanline
        let mut y_offset: usize = self.get_vcount() as usize;
        let v_count = self.get_vcount();
        let mut bg_mode: u8 = 0;

        {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };

            // Add BG-specific Y offset
            if index == 2 {
                y_offset += (engine.bg2y >> 8) as usize;
            } else {
                y_offset += (engine.bg3y >> 8) as usize;
            }

            // Calculate base address for the BG tiles
            base += (((engine.bgcnt[index] >> 8) & 0x1F) as u32) * 1024 * 16;

            // Determine BG mode (2-bit value)
            if (engine.bgcnt[index] & (1 << 2)) != 0 {
                bg_mode += 1;
            }
            if (engine.bgcnt[index] & (1 << 7)) != 0 {
                bg_mode += 2;
            }
        }

        match bg_mode {
            0 | 1 => {
                // Modes 0-1: Text/Tile modes
                self.draw_ext_text(index, is_engine_a);
            }

            2 => {
                // Mode 2: Rotscale 256-color bitmap
                for i in 0..PIXELS_PER_LINE {
                    let address = base + i as u32 + ((v_count as u32) * (PIXELS_PER_LINE as u32));

                    let color_index = match is_engine_a {
                        true => self.read_bga_u8(address),
                        false => self.read_bgb_u8(address),
                    };

                    if color_index == 0 {
                        continue;
                    }

                    // Convert palette index to RGB
                    let address = (color_index * 2) as u32;
                    let color = match is_engine_a {
                        true => read_palette_value(&self.palette_upper, address),
                        false => read_palette_value(&self.palette_lower, address),
                    };

                    let r = ((color & 0x1F) << 3) as u32;
                    let g = (((color >> 5) & 0x1F) << 3) as u32;
                    let b = (((color >> 10) & 0x1F) << 3) as u32;

                    let true_color = 0xFF000000 | (r << 16) | (g << 8) | b;

                    // NOTE: index <= 4
                    let address = i + (v_count as usize * PIXELS_PER_LINE);

                    let engine = match is_engine_a {
                        true => &mut self.engine_upper,
                        false => &mut self.engine_lower,
                    };
                    engine.framebuffer[address] = true_color;
                    engine.final_bg_priority[i] = (engine.bgcnt[index] & 0x3) as u8;
                }
            }

            3 => {
                // Mode 3: Direct color bitmap
                for i in 0..PIXELS_PER_LINE {
                    let ds_color: u16 = if is_engine_a {
                        self.read_bga_u16(
                            base + (i as u32 * 2) + (y_offset as u32 * PIXELS_PER_LINE as u32 * 2),
                        )
                    } else {
                        self.read_bgb_u16(
                            base + (i as u32 * 2) + (y_offset as u32 * PIXELS_PER_LINE as u32 * 2),
                        )
                    };

                    // Only consider colors with bit 15 set
                    if (ds_color & (1 << 15)) == 0 {
                        continue;
                    }

                    let r = ((ds_color & 0x1F) << 3) as u32;
                    let g = (((ds_color >> 5) & 0x1F) << 3) as u32;
                    let b = (((ds_color >> 10) & 0x1F) << 3) as u32;

                    let color = 0xFF000000 | (r << 16) | (g << 8) | b;

                    // NOTE: index <= 4
                    let address = i + (v_count as usize * PIXELS_PER_LINE);

                    let engine = match is_engine_a {
                        true => &mut self.engine_upper,
                        false => &mut self.engine_lower,
                    };
                    engine.framebuffer[address] = color;
                    engine.final_bg_priority[i] = (engine.bgcnt[index] & 0x3) as u8;
                }
            }

            _ => {
                panic!("Unrecognized extended mode {}", bg_mode);
            }
        }
    }
}
