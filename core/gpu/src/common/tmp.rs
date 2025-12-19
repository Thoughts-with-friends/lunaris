use crate::common::{Gpu, bytes_to_palette, read_palette_value};
use mem_const::*;

// let engine = match is_engine_a {
//     true => &mut self.eng_a,
//     false => &mut self.eng_b,
// };

impl Gpu {
    /// Draws one scanline.
    pub fn draw_scanline(&mut self) {
        let is_engine_a = if self.powcnt1.engine_a {
            true
        } else if self.powcnt1.engine_b {
            false
        } else {
            return;
        };

        let line_start = self.get_vcount() as usize * PIXELS_PER_LINE;

        // Clear scanline
        for i in 0..PIXELS_PER_LINE {
            let engine = match is_engine_a {
                true => &mut self.eng_a,
                false => &mut self.eng_b,
            };
            engine.framebuffer[line_start + i] = 0xFF000000;
            engine.front_framebuffer[line_start + i] = 0xFF000000;
        }

        // Reset BG priority
        let engine = match is_engine_a {
            true => &mut self.eng_a,
            false => &mut self.eng_b,
        };
        engine.final_bg_priority.fill(0xFF);

        // Draw backdrop
        self.draw_backdrop(is_engine_a);

        // Window mask
        if engine.dispcnt.display_win0
            || engine.dispcnt.display_win1
            || engine.dispcnt.obj_win_display
        {
            engine.get_window_mask();
        } else {
            engine.window_mask.fill(0xFF);
        }

        // Draw BG layers by priority
        for priority in (0..=3).rev() {
            for bg_index in 0..4 {
                if !Config::bg_enable[bg_index] {
                    continue;
                }
                let engine = match is_engine_a {
                    true => &mut self.eng_a,
                    false => &mut self.eng_b,
                };
                if (engine.bgcnt[bg_index] & 0x3) as usize != priority {
                    continue;
                }
                if !engine.dispcnt.display_bg[bg_index] {
                    continue;
                }

                match bg_index {
                    0 => {
                        if is_engine_a && engine.dispcnt.bg_3d {
                            engine.draw_3d_scanline(
                                &mut engine.framebuffer,
                                &engine.final_bg_priority,
                                priority,
                            );
                        } else {
                            engine.draw_bg_txt(0, is_engine_a);
                        }
                    }
                    1 => engine.draw_bg_txt(1, is),
                    2 => match engine.dispcnt.bg_mode {
                        0 | 1 | 3 => engine.draw_bg_txt(2, is_engine_a),
                        5 => engine.draw_bg_ext(2, is_engine_a),
                        _ => {}
                    },
                    3 => match engine.dispcnt.bg_mode {
                        0 => engine.draw_bg_txt(3),
                        3 | 4 | 5 => self.draw_bg_ext(3),
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        // Draw sprites
        if self.dispcnt.display_obj {
            self.draw_sprites();
        }

        // Handle blending effects
        self.handle_bldcnt_effects();

        // Display mode handling
        match self.dispcnt.display_mode {
            0 => {
                for i in 0..PIXELS_PER_LINE {
                    self.front_framebuffer[line_start + i] = 0xFFF3F3F3;
                }
            }
            1 => {
                for i in 0..PIXELS_PER_LINE {
                    self.front_framebuffer[line_start + i] = self.framebuffer[line_start + i];
                }
            }
            2 => {
                let vram = self.gpu.get_vram_block(self.dispcnt.vram_block);
                for x in 0..PIXELS_PER_LINE {
                    let ds_color = vram[line_start + x];
                    let r = (ds_color & 0x1F) << 3;
                    let g = ((ds_color >> 5) & 0x1F) << 3;
                    let b = ((ds_color >> 10) & 0x1F) << 3;
                    self.front_framebuffer[line_start + x] = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
            _ => {}
        }

        // Capture (DISPCAPCNT)
        if self.engine_a && self.dispcapcnt.enable_busy {
            let (x_size, y_size) = match self.dispcapcnt.capture_size {
                0 => (128, 128),
                1 => (256, 64),
                2 => (256, 128),
                3 => (256, 192),
                _ => (0, 0),
            };

            if self.gpu.get_vcount() < y_size {
                let read_offset = match self.dispcnt.display_mode {
                    2 => 0,
                    _ => self.dispcapcnt.vram_read_offset as usize * 0x4000,
                } + line_start;

                let write_offset = self.dispcapcnt.vram_write_offset as usize * 0x4000 + line_start;

                let vram_dest = self.gpu.get_vram_block(self.dispcapcnt.vram_write_block);

                let vram_src = self.gpu.get_vram_block(self.dispcnt.vram_block);

                for x in 0..x_size {
                    let source_a = self.framebuffer[line_start + x];
                    let source_b = vram_src[(read_offset + x) & 0xFFFF] as u32;

                    let (ra, ga, ba) = (
                        (source_a >> 19) & 0x1F,
                        (source_a >> 11) & 0x1F,
                        (source_a >> 3) & 0x1F,
                    );
                    let (rb, gb, bb) = (
                        source_b & 0x1F,
                        (source_b >> 5) & 0x1F,
                        (source_b >> 10) & 0x1F,
                    );

                    let (mut rd, mut gd, mut bd) = match self.dispcapcnt.capture_source {
                        0 => (ra, ga, ba),
                        1 => (rb, gb, bb),
                        2 | 3 => (
                            (ra * self.dispcapcnt.eva + rb * self.dispcapcnt.evb) / 16,
                            (ga * self.dispcapcnt.eva + gb * self.dispcapcnt.evb) / 16,
                            (ba * self.dispcapcnt.eva + bb * self.dispcapcnt.evb) / 16,
                        ),
                        _ => {
                            println!(
                                "Unrecognized capture source {}",
                                self.dispcapcnt.capture_source
                            );
                            (0, 0, 0)
                        }
                    };

                    rd = rd.min(0x1F);
                    gd = gd.min(0x1F);
                    bd = bd.min(0x1F);

                    vram_dest[(write_offset + x) & 0xFFFF] =
                        rd | (gd << 5) | (bd << 10) | (1 << 15);
                }
            }
        }

        // Apply master brightness
        let bright_mode = self.master_bright >> 14;
        let mut bright_factor = (self.master_bright & 0x1F) as f32;
        if bright_factor > 16.0 {
            bright_factor = 16.0;
        }

        match bright_mode {
            1 => {
                // Increase brightness
                for i in 0..PIXELS_PER_LINE {
                    let idx = line_start + i;
                    let mut r = ((self.front_framebuffer[idx] >> 16) & 0xFF) as f32;
                    let mut g = ((self.front_framebuffer[idx] >> 8) & 0xFF) as f32;
                    let mut b = (self.front_framebuffer[idx] & 0xFF) as f32;

                    r += ((63.0 * 4.0 - r) * (bright_factor / 16.0));
                    g += ((63.0 * 4.0 - g) * (bright_factor / 16.0));
                    b += ((63.0 * 4.0 - b) * (bright_factor / 16.0));

                    self.front_framebuffer[idx] =
                        0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                }
            }
            2 => {
                // Decrease brightness
                for i in 0..PIXELS_PER_LINE {
                    let idx = line_start + i;
                    let mut r = ((self.front_framebuffer[idx] >> 16) & 0xFF) as f32;
                    let mut g = ((self.front_framebuffer[idx] >> 8) & 0xFF) as f32;
                    let mut b = (self.front_framebuffer[idx] & 0xFF) as f32;

                    r -= r * (bright_factor / 16.0);
                    g -= g * (bright_factor / 16.0);
                    b -= b * (bright_factor / 16.0);

                    self.front_framebuffer[idx] =
                        0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                }
            }
            _ => {}
        }
    }
}
