// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu.hpp
//!
//! ## From the engine's line to the screen
//!
//! ```text
//!   compose_scanline() row -----+
//!   VRAM bank (LCDC) row -------+--> DISPCNT bits 16-17     --> MASTER_BRIGHT --> screen
//!   main memory FIFO -----------+    0 off (white)
//!                                    1 graphics (normal)
//!                                    2 VRAM display
//!                                    3 main memory (not emulated)
//!
//!   display capture (engine A, DISPCAPCNT bit 31 = start):
//!     source A: the 2D+3D line, or 3D only ----+
//!                                              +--> A*EVA/16 + B*EVB/16 --> VRAM bank
//!     source B: a VRAM bank (read offset) -----+    (or A only / B only)    (write offset)
//!     lets games reuse a finished frame: motion blur, render-to-texture,
//!     or 3D on both screens (capture one frame, show it from VRAM)
//!
//!   MASTER_BRIGHT (bits 14-15 mode, bits 0-4 factor F, max 16):
//!     mode 1 up:    c + (max - c) * F/16    fades toward white
//!     mode 2 down:  c - c * F/16            fades toward black
//! ```
//!
//! GBATEK: [DS Video Capture and Main Memory Display Mode](https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode),
//! [DS Video Stuff](https://problemkaputt.de/gbatek.htm#dsvideostuff)
use ds_mem_addr::*;

use crate::Gpu;

impl Gpu {
    /// Draws the current scanline for every powered-on 2D engine.
    ///
    /// ```text
    /// from: ds_emu gpu_handle_event(), start of HBlank on lines 0..191
    ///       (skipped on frame-skipped frames)
    ///   |
    ///   v
    /// draw_scanline()
    ///   POWCNT1 engine A on -> draw_scanline_for_engine(A)
    ///   POWCNT1 engine B on -> draw_scanline_for_engine(B)
    ///   |
    ///   v
    /// to:   front_framebuffer rows, copied out by get_upper/lower_frame()
    /// ```
    ///
    /// Matches CorgiDS `GPU::draw_scanline()`, which calls
    /// `eng_A.draw_scanline()` and `eng_B.draw_scanline()` unconditionally
    /// (each gated only by its own POWCNT1 enable bit). The previous
    /// `if engine_upper { .. } else if engine_lower { .. }` chain here
    /// rendered at most one engine per scanline: whichever engine were
    /// enabled first always won, so the other engine's framebuffer was never
    /// written and stayed at its cleared/default color for the whole run.
    pub fn draw_scanline(&mut self) {
        if self.power_control_reg.engine_upper {
            self.draw_scanline_for_engine(true);
        }
        if self.power_control_reg.engine_lower {
            self.draw_scanline_for_engine(false);
        }
    }

    /// Renders one scanline for a single 2D engine (Engine A if
    /// `is_engine_a`, otherwise Engine B).
    ///
    /// ```text
    /// from: draw_scanline()
    ///   |
    ///   v
    /// draw_scanline_for_engine(engine)
    ///   1. compose_scanline()       BGs + OBJs + windows + blending
    ///                               -> framebuffer row
    ///   2. DISPCNT display mode:
    ///        0 off       -> white row
    ///        1 normal    -> framebuffer row
    ///        2 VRAM      -> LCDC bank row (BGR555 -> RGB888)
    ///        (3 main memory FIFO: not implemented)
    ///      -> front_framebuffer row
    ///   3. engine A capture (DISPCAPCNT): blend source A (2D/3D) and B
    ///      (VRAM) into the capture bank
    ///   4. MASTER_BRIGHT fade up / down on the row
    /// ```
    fn draw_scanline_for_engine(&mut self, is_engine_a: bool) {
        let line_start = self.get_vcount() as usize * PIXELS_PER_LINE;

        // Layers, windows and color effects -> `framebuffer` row.
        self.compose_scanline(is_engine_a);

        let display_mode = {
            let engine = match is_engine_a {
                true => &self.engine_upper,
                false => &self.engine_lower,
            };
            engine.dispcnt.display_mode
        };

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
                        // Display mode 0 ("Display Off") outputs a fixed
                        // near-white color per GBATEK "DS Video / DISPCNT",
                        // matching CorgiDS `front_framebuffer[i + line] =
                        // 0xFFF3F3F3;`. This had been left pointing at a
                        // leftover debug magenta value instead.
                        engine.front_framebuffer[line_start + i] = 0xFFF3_F3F3;
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
                    // safe access (mirrors the guard used by display modes 0/1 above)
                    if (line_start + x) >= PIXELS_PER_LINE * SCANLINES {
                        break;
                    }

                    let ds_color = {
                        let vram = self.get_vram_block(vram_block);
                        vram[line_start + x]
                    };

                    // LCD Color Palettes: https://problemkaputt.de/gbatek.htm#lcdcolorpalettes
                    let r = ((ds_color & 0x1F) << 3) as u32; // red (bits 0-4) to 8 bits
                    let g = (((ds_color >> 5) & 0x1F) << 3) as u32; // green (bits 5-9) to 8 bits
                    let b = (((ds_color >> 10) & 0x1F) << 3) as u32; // blue (bits 10-14) to 8 bits

                    let engine = match is_engine_a {
                        true => &mut self.engine_upper,
                        false => &mut self.engine_lower,
                    };
                    engine.front_framebuffer[line_start + x] = 0xFF000000 | (r << 16) | (g << 8) | b // opaque ARGB8888
                }
            }
            _ => {}
        }

        let (enable_a_busy, capture_size) = {
            let engine = match is_engine_a {
                true => &mut self.engine_upper,
                false => &mut self.engine_lower,
            };
            (is_engine_a && engine.dispcapcnt.enable_busy, engine.dispcapcnt.capture_size)
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
                        // DS Video Capture and Main Memory Display Mode: https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode
                        _ => engine.dispcapcnt.vram_read_offset as usize * 0x4000, // DISPCAPCNT VRAM read offset (bits 26-27) in 32K steps (16K halfwords)
                    } + line_start;

                    let write_offset =
                        engine.dispcapcnt.vram_write_offset as usize * 0x4000 + line_start; // DISPCAPCNT VRAM write offset (bits 18-19) in 32K steps (16K halfwords)

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
                        vram_src[(read_offset + x) & 0xFFFF] as u32 // offset wraps within the 128K bank (64K halfwords)
                    };

                    let (ra, ga, ba) = (
                        ((source_a >> 19) & 0x1F) as u16, // source A red
                        ((source_a >> 11) & 0x1F) as u16, // source A green
                        ((source_a >> 3) & 0x1F) as u16,  // source A blue
                    );
                    let (rb, gb, bb) = (
                        (source_b & 0x1F) as u16,         // source B red (bits 0-4)
                        ((source_b >> 5) & 0x1F) as u16,  // source B green (bits 5-9)
                        ((source_b >> 10) & 0x1F) as u16, // source B blue (bits 10-14)
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
                            #[cfg(feature = "log")]
                            log::error!(
                                "Unrecognized capture source {}",
                                engine.dispcapcnt.capture_source
                            );
                            (0, 0, 0)
                        }
                    };

                    rd = rd.min(0x1F); // clamp to 5 bits
                    gd = gd.min(0x1F); // clamp to 5 bits
                    bd = bd.min(0x1F); // clamp to 5 bits

                    let vram_dest = self.get_vram_block_mut(vram_write_block);
                    vram_dest[(write_offset + x) & 0xFFFF] = // offset wraps within the 128K bank (64K halfwords)
                        rd | (gd << 5) | (bd << 10) | (1 << 15); // BGR555 with alpha bit 15 set
                }
            }
        }

        if is_engine_a && let Some(hd) = self.hd_lines.get_mut(self.vertical_count as usize) {
            *hd = crate::display::hires::HdLine {
                graphics: display_mode == 1,
                master_bright: self.engine_upper.master_bright,
                bg0_hofs: self.engine_upper.bghofs[0],
            };
        }

        let engine = match is_engine_a {
            true => &mut self.engine_upper,
            false => &mut self.engine_lower,
        };

        // Apply master brightness
        let bright_mode = engine.master_bright >> 14;
        // DS Video Stuff: https://problemkaputt.de/gbatek.htm#dsvideostuff
        let mut bright_factor = (engine.master_bright & 0x1F) as f32; // MASTER_BRIGHT factor (bits 0-4)
        if bright_factor > 16.0 {
            bright_factor = 16.0;
        }

        match bright_mode {
            1 => {
                // Increase brightness
                for i in 0..PIXELS_PER_LINE {
                    let idx = line_start + i;
                    let mut r = ((engine.front_framebuffer[idx] >> 16) & 0xFF) as f32; // red channel
                    let mut g = ((engine.front_framebuffer[idx] >> 8) & 0xFF) as f32; // green channel
                    let mut b = (engine.front_framebuffer[idx] & 0xFF) as f32; // blue channel

                    r += (255.0 - r) * (bright_factor / 16.0);
                    g += (255.0 - g) * (bright_factor / 16.0);
                    b += (255.0 - b) * (bright_factor / 16.0);

                    engine.front_framebuffer[idx] =
                        0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32); // opaque ARGB8888
                }
            }
            2 => {
                // Decrease brightness
                for i in 0..PIXELS_PER_LINE {
                    let idx = line_start + i;
                    let mut r = ((engine.front_framebuffer[idx] >> 16) & 0xFF) as f32; // red channel
                    let mut g = ((engine.front_framebuffer[idx] >> 8) & 0xFF) as f32; // green channel
                    let mut b = (engine.front_framebuffer[idx] & 0xFF) as f32; // blue channel

                    r -= r * (bright_factor / 16.0);
                    g -= g * (bright_factor / 16.0);
                    b -= b * (bright_factor / 16.0);

                    engine.front_framebuffer[idx] =
                        0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32); // opaque ARGB8888
                }
            }
            _ => {}
        }
    }
}
