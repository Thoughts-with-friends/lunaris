// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
use crate::gpu_root::Gpu;
use lunaris_ds_mem_const::*;

impl Gpu {
    pub fn draw_3d_scanline(&mut self, is_engine_a: bool, bg_priority: u8) {
        self.render_scanline(is_engine_a, bg_priority);
    }

    /// Draws one scanline.
    pub fn draw_scanline(&mut self) {
        let is_engine_a = if self.power_control_reg.engine_upper {
            true
        } else if self.power_control_reg.engine_lower {
            false
        } else {
            return;
        };

        let line_start = self.get_vcount() as usize * PIXELS_PER_LINE;

        // Clear scanline
        for i in 0..PIXELS_PER_LINE {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };

            // safe access
            if (line_start + i) < PIXELS_PER_LINE * SCANLINES {
                engine.framebuffer[line_start + i] = 0xFF000000;
                engine.front_framebuffer[line_start + i] = 0xFF000000;
            }
        }

        {
            // Reset BG priority
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };
            engine.final_bg_priority.fill(0xFF);
        }

        // Draw backdrop
        self.draw_backdrop(is_engine_a);

        let window_masked = {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };
            engine.dispcnt.display_win0
                || engine.dispcnt.display_win1
                || engine.dispcnt.obj_win_display
        };

        // Window mask
        if window_masked {
            self.get_window_mask(is_engine_a);
        } else {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };
            engine.window_mask.fill(0xFF);
        }

        // Draw BG layers by priority
        for priority in (0..=3).rev() {
            let bg_enable = {
                let engine = match is_engine_a {
                    true => &mut self.engine_upper,
                    false => &mut self.engine_lower,
                };
                engine.bgcnt
            };
            for (bg_index, bg_enable_value) in bg_enable.iter().enumerate() {
                if *bg_enable_value == 0 {
                    continue;
                }
                let engine = match is_engine_a {
                    true => &mut self.engine_upper,
                    false => &mut self.engine_lower,
                };
                if (engine.bgcnt[bg_index] & 0x3) as u8 != priority {
                    continue;
                }

                if !match bg_index {
                    0 => engine.dispcnt.display_bg0, // => bool
                    1 => engine.dispcnt.display_bg1,
                    2 => engine.dispcnt.display_bg2,
                    3 => engine.dispcnt.display_bg3,
                    _ => unreachable!(),
                } {
                    continue;
                }

                match bg_index {
                    0 => {
                        if is_engine_a && engine.dispcnt.bg_3d {
                            self.draw_3d_scanline(is_engine_a, priority);
                        } else {
                            self.draw_bg_txt(0, is_engine_a);
                        }
                    }
                    1 => self.draw_bg_txt(1, is_engine_a),
                    2 => match engine.dispcnt.bg_mode {
                        0 | 1 | 3 => self.draw_bg_txt(2, is_engine_a),
                        5 => self.draw_bg_ext(2, is_engine_a),
                        _ => {}
                    },
                    3 => match engine.dispcnt.bg_mode {
                        0 | 3 | 4 | 5 => self.draw_bg_ext(3, is_engine_a),
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        let (display_obj, display_mode) = {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };
            (engine.dispcnt.display_obj, engine.dispcnt.display_mode)
        };

        // Draw sprites
        if display_obj {
            self.draw_sprites(is_engine_a);
        }

        // Handle blending effects
        self.handle_bldcnt_effects();

        // Display mode handling
        match display_mode {
            0 => {
                for i in 0..PIXELS_PER_LINE {
                    let engine = match is_engine_a {
                        true => &mut self.engine_upper,
                        false => &mut self.engine_lower,
                    };

                    // safe access
                    if (line_start + i) < PIXELS_PER_LINE * SCANLINES {
                        engine.front_framebuffer[line_start + i] = 0xFFF3F3F3;
                    }
                }
            }
            1 => {
                for i in 0..PIXELS_PER_LINE {
                    let engine = match is_engine_a {
                        true => &mut self.engine_upper,
                        false => &mut self.engine_lower,
                    };

                    // safe access
                    if (line_start + i) < PIXELS_PER_LINE * SCANLINES {
                        engine.front_framebuffer[line_start + i] =
                            engine.framebuffer[line_start + i];
                    }
                }
            }
            2 => {
                let vram_block = {
                    let engine = match is_engine_a {
                        true => &mut self.engine_upper,
                        false => &mut self.engine_lower,
                    };
                    engine.dispcnt.vram_block
                };
                for x in 0..PIXELS_PER_LINE {
                    let ds_color = {
                        let vram = self.get_vram_block(vram_block);
                        vram[line_start + x]
                    };

                    let r = ((ds_color & 0x1F) << 3) as u32;
                    let g = (((ds_color >> 5) & 0x1F) << 3) as u32;
                    let b = (((ds_color >> 10) & 0x1F) << 3) as u32;

                    let engine = match is_engine_a {
                        true => &mut self.engine_upper,
                        false => &mut self.engine_lower,
                    };
                    engine.front_framebuffer[line_start + x] = 0xFF000000 | (r << 16) | (g << 8) | b
                }
            }
            _ => {}
        }

        let (enable_a_busy, capture_size) = {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };
            (
                is_engine_a && engine.dispcapcnt.enable_busy,
                engine.dispcapcnt.capture_size,
            )
        };

        // Capture (DISPCAPCNT)
        if enable_a_busy {
            let (x_size, y_size) = match capture_size {
                0 => (128, 128),
                1 => (256, 64),
                2 => (256, 128),
                3 => (256, 192),
                _ => (0, 0),
            };

            if self.get_vcount() < y_size {
                let (read_offset, write_offset, vram_write_block, vram_block) = {
                    let engine = match is_engine_a {
                        true => &mut self.engine_upper,
                        false => &mut self.engine_lower,
                    };
                    let read_offset = match engine.dispcnt.display_mode {
                        2 => 0,
                        _ => engine.dispcapcnt.vram_read_offset as usize * 0x4000,
                    } + line_start;

                    let write_offset =
                        engine.dispcapcnt.vram_write_offset as usize * 0x4000 + line_start;

                    (
                        read_offset,
                        write_offset,
                        engine.dispcapcnt.vram_write_block as i32,
                        engine.dispcnt.vram_block,
                    )
                };

                for x in 0..x_size {
                    let engine = match is_engine_a {
                        true => &self.engine_upper,
                        false => &self.engine_lower,
                    };
                    let source_a = engine.framebuffer[line_start + x];
                    let source_b = {
                        let vram_src = self.get_vram_block(vram_block);
                        vram_src[(read_offset + x) & 0xFFFF] as u32
                    };

                    let (ra, ga, ba) = (
                        ((source_a >> 19) & 0x1F) as u16,
                        ((source_a >> 11) & 0x1F) as u16,
                        ((source_a >> 3) & 0x1F) as u16,
                    );
                    let (rb, gb, bb) = (
                        (source_b & 0x1F) as u16,
                        ((source_b >> 5) & 0x1F) as u16,
                        ((source_b >> 10) & 0x1F) as u16,
                    );

                    let (mut rd, mut gd, mut bd) = match engine.dispcapcnt.capture_source {
                        0 => (ra, ga, ba),
                        1 => (rb, gb, bb),
                        2 | 3 => {
                            let eva = engine.dispcapcnt.eva as u16;
                            let evb = engine.dispcapcnt.evb as u16;
                            (
                                (ra * eva + rb * evb) / 16,
                                (ga * eva + gb * evb) / 16,
                                (ba * eva + bb * evb) / 16,
                            )
                        }
                        _ => {
                            #[cfg(feature = "tracing")]
                            tracing::error!(
                                "Unrecognized capture source {}",
                                engine.dispcapcnt.capture_source
                            );
                            (0, 0, 0)
                        }
                    };

                    rd = rd.min(0x1F);
                    gd = gd.min(0x1F);
                    bd = bd.min(0x1F);

                    let vram_dest = self.get_vram_block_mut(vram_write_block);
                    vram_dest[(write_offset + x) & 0xFFFF] =
                        rd | (gd << 5) | (bd << 10) | (1 << 15);
                }
            }
        }

        let engine = match is_engine_a {
            true => &mut self.engine_upper,
            false => &mut self.engine_lower,
        };

        // Apply master brightness
        let bright_mode = engine.master_bright >> 14;
        let mut bright_factor = (engine.master_bright & 0x1F) as f32;
        if bright_factor > 16.0 {
            bright_factor = 16.0;
        }

        match bright_mode {
            1 => {
                // Increase brightness
                for i in 0..PIXELS_PER_LINE {
                    let idx = line_start + i;
                    let mut r = ((engine.front_framebuffer[idx] >> 16) & 0xFF) as f32;
                    let mut g = ((engine.front_framebuffer[idx] >> 8) & 0xFF) as f32;
                    let mut b = (engine.front_framebuffer[idx] & 0xFF) as f32;

                    r += (63.0 * 4.0 - r) * (bright_factor / 16.0);
                    g += (63.0 * 4.0 - g) * (bright_factor / 16.0);
                    b += (63.0 * 4.0 - b) * (bright_factor / 16.0);

                    engine.front_framebuffer[idx] =
                        0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                }
            }
            2 => {
                // Decrease brightness
                for i in 0..PIXELS_PER_LINE {
                    let idx = line_start + i;
                    let mut r = ((engine.front_framebuffer[idx] >> 16) & 0xFF) as f32;
                    let mut g = ((engine.front_framebuffer[idx] >> 8) & 0xFF) as f32;
                    let mut b = (engine.front_framebuffer[idx] & 0xFF) as f32;

                    r -= r * (bright_factor / 16.0);
                    g -= g * (bright_factor / 16.0);
                    b -= b * (bright_factor / 16.0);

                    engine.front_framebuffer[idx] =
                        0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                }
            }
            _ => {}
        }
    }
}
