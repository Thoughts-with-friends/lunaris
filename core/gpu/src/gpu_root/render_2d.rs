// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpueng.cpp
//!
//! See: https://github.com/PSI-Rockin/CorgiDS/blob/0040fccf587ae04c041db562ced4c07f3d937594/src/gpueng.cpp#L363
//!
//! CorgiDS was calling GPU methods in Engine2D, but this caused a circular reference.
//! To avoid this, we've implemented the method in the parent here.
use crate::gpu_root::{Gpu, bytes_to_palette, read_palette_value};
use crate::vram::Region;
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
        // Nothing impl in CorgiDS
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

            // safe access
            if (base + x) < PIXELS_PER_LINE * SCANLINES {
                engine.framebuffer[base + x] = color;
            }
        }
    }

    /// Draws a text background layer for the current scanline.
    ///
    /// `index` must be 0..=3. Follows GBATEK "DS Video BG Modes / Control":
    /// the map is made of 32x32-tile screen blocks of 0x800 bytes (a 512-wide
    /// map has its right half in the next block; a 512-tall one has its lower
    /// half one block (256x512) or two blocks (512x512) further), tiles are
    /// 4bpp (16 palettes of 16 colors) or 8bpp (one 256-color palette, or the
    /// extended palette slot of this BG when DISPCNT bit 30 is set).
    pub fn draw_bg_txt(&mut self, index: usize, is_engine_a: bool) {
        let region = if is_engine_a {
            Region::BgA
        } else {
            Region::BgB
        };
        let line = self.get_vcount() as usize;
        if line >= SCANLINES {
            return;
        }

        let engine = if is_engine_a {
            &self.engine_upper
        } else {
            &self.engine_lower
        };
        let bgcnt = engine.bgcnt[index];
        let priority = (bgcnt & 0x3) as u8;
        let hofs = u32::from(engine.bghofs[index]);
        let vofs = u32::from(engine.bgvofs[index]);
        let (char_base, screen_base) = if is_engine_a {
            (
                engine.dispcnt.char_base as u32 * 0x1_0000,
                engine.dispcnt.screen_base as u32 * 0x1_0000,
            )
        } else {
            (0, 0)
        };
        let char_base = char_base + u32::from((bgcnt >> 2) & 0xF) * 0x4000;
        let screen_base = screen_base + u32::from((bgcnt >> 8) & 0x1F) * 0x800;
        let color_256 = bgcnt & (1 << 7) != 0;
        let size = (bgcnt >> 14) & 0x3;
        let ext_palette = color_256 && engine.dispcnt.bg_extended_palette;
        // BG0/BG1 may use extended palette slot 2/3 instead (BGCNT bit 13).
        let ext_slot = if index < 2 && bgcnt & (1 << 13) != 0 {
            index + 2
        } else {
            index
        } as u32;

        let width_mask = if size & 1 != 0 { 511 } else { 255 };
        let height_mask = if size & 2 != 0 { 511 } else { 255 };
        let y = (vofs + line as u32) & height_mask;
        let fine_y = y & 7;
        let mut row_base = screen_base + ((y >> 3) & 31) * 64;
        if y >= 256 {
            row_base += if size == 3 { 0x1000 } else { 0x800 };
        }

        let palette = if is_engine_a {
            &self.palette_upper
        } else {
            &self.palette_lower
        };
        let scanline = line * PIXELS_PER_LINE;

        // Tile state, refetched whenever the map column changes.
        let mut cached_column = u32::MAX;
        let mut entry = 0u16;

        for px in 0..PIXELS_PER_LINE {
            let engine = if is_engine_a {
                &self.engine_upper
            } else {
                &self.engine_lower
            };
            if engine.window_mask[px] & (1 << index) == 0 {
                continue;
            }

            let x = (hofs + px as u32) & width_mask;
            let column = x >> 3;
            if column != cached_column {
                cached_column = column;
                let block = if x >= 256 { 0x800 } else { 0 };
                entry = self
                    .vram
                    .read_u16(region, row_base + block + (column & 31) * 2);
            }

            let tile = u32::from(entry & 0x3FF);
            let tx = if entry & (1 << 10) != 0 {
                7 - (x & 7)
            } else {
                x & 7
            };
            let ty = if entry & (1 << 11) != 0 {
                7 - fine_y
            } else {
                fine_y
            };
            let pal_bank = u32::from(entry >> 12);

            let color = if color_256 {
                let ci = u32::from(
                    self.vram
                        .read_u8(region, char_base + tile * 64 + ty * 8 + tx),
                );
                if ci == 0 {
                    continue;
                }
                if ext_palette {
                    let ext = if is_engine_a {
                        Region::BgExtPalA
                    } else {
                        Region::BgExtPalB
                    };
                    self.vram
                        .read_u16(ext, ext_slot * 0x2000 + pal_bank * 512 + ci * 2)
                } else {
                    read_palette_value(palette, ci * 2)
                }
            } else {
                let byte = self
                    .vram
                    .read_u8(region, char_base + tile * 32 + ty * 4 + tx / 2);
                let ci = u32::from(if tx & 1 != 0 { byte >> 4 } else { byte & 0xF });
                if ci == 0 {
                    continue;
                }
                read_palette_value(palette, (pal_bank * 16 + ci) * 2)
            };

            let r = u32::from(color & 0x1F) << 3;
            let g = u32::from((color >> 5) & 0x1F) << 3;
            let b = u32::from((color >> 10) & 0x1F) << 3;
            let engine = if is_engine_a {
                &mut self.engine_upper
            } else {
                &mut self.engine_lower
            };
            engine.framebuffer[scanline + px] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
            engine.final_bg_priority[px] = priority;
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
