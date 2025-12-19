pub struct Gpu2DEngine<'a> {
    // Parent GPU
    pub gpu: *mut GPU,
    pub engine_a: bool,

    // Framebuffers
    pub framebuffer: &'a mut [u32],
    pub front_framebuffer: &'a mut [u32],

    // Registers
    pub dispcnt: DISPCNT,
    pub dispcapcnt: DISPCAPCNT,

    pub bgcnt: [u16; 4],
    pub bghofs: [u16; 4],
    pub bgvofs: [u16; 4],

    pub bg2p: [u16; 4],
    pub bg3p: [u16; 4],
    pub bg2p_internal: [u16; 4],
    pub bg3p_internal: [u16; 4],

    pub bg2x: u32,
    pub bg2y: u32,
    pub bg3x: u32,
    pub bg3y: u32,

    pub bg2x_internal: u32,
    pub bg2y_internal: u32,
    pub bg3x_internal: u32,
    pub bg3y_internal: u32,

    pub win0h: u16,
    pub win1h: u16,
    pub win0v: u16,
    pub win1v: u16,

    pub winin: WININ,
    pub winout: WINOUT,

    pub bldcnt: BLDCNT,
    pub bldalpha: u16,
    pub bldy: u8,
    pub master_bright: u16,

    // Scanline buffers
    pub window_mask: [u8; PIXELS_PER_LINE],
    pub final_bg_priority: [u8; PIXELS_PER_LINE * 2],
    pub sprite_scanline: [u32; PIXELS_PER_LINE * 2],

    pub win0_active: bool,
    pub win1_active: bool,
}

impl<'a> Gpu2DEngine<'a> {
    /// Handle VBLANK start
    pub fn vblank_start(&mut self) {
        for i in 0..4 {
            self.bg2p_internal[i] = self.bg2p[i];
            self.bg3p_internal[i] = self.bg3p[i];
        }

        self.bg2x_internal = self.bg2x;
        self.bg2y_internal = self.bg2y;
        self.bg3x_internal = self.bg3x;
        self.bg3y_internal = self.bg3y;

        self.dispcapcnt.enable_busy = false;
    }

    /// Draw backdrop color
    pub fn draw_backdrop(&mut self) {
        let palette = unsafe { (*self.gpu).get_palette(self.engine_a) };

        let y = unsafe { (*self.gpu).get_vcount() } as usize;
        let base = y * PIXELS_PER_LINE;

        for x in 0..PIXELS_PER_LINE {
            let c = palette[0];
            let r = ((c & 0x1F) << 3) as u32;
            let g = (((c >> 5) & 0x1F) << 3) as u32;
            let b = (((c >> 10) & 0x1F) << 3) as u32;

            self.framebuffer[base + x] = 0xFF000000 | (r << 16) | (g << 8) | b;
        }
    }

    /// Compute window mask for current scanline
    pub fn get_window_mask(&mut self) {
        let line = unsafe { (*self.gpu).get_vcount() & 0xFF };

        let y1_0 = (self.win0v >> 8) as i32;
        let y2_0 = (self.win0v & 0xFF) as i32;
        let y1_1 = (self.win1v >> 8) as i32;
        let y2_1 = (self.win1v & 0xFF) as i32;

        if line == y1_0 {
            self.win0_active = true;
        } else if line == y2_0 {
            self.win0_active = false;
        }

        if line == y1_1 {
            self.win1_active = true;
        } else if line == y2_1 {
            self.win1_active = false;
        }

        let winout = self.get_winout() as u8;
        for i in 0..PIXELS_PER_LINE {
            self.window_mask[i] = winout;
        }

        if self.dispcnt.display_win1 && self.win1_active {
            let x1 = (self.win1h >> 8) as usize;
            let x2 = (self.win1h & 0xFF) as usize;
            let mask = (self.get_winin() >> 8) as u8;
            for x in x1..x2 {
                self.window_mask[x] = mask;
            }
        }

        if self.dispcnt.display_win0 && self.win0_active {
            let x1 = (self.win0h >> 8) as usize;
            let x2 = (self.win0h & 0xFF) as usize;
            let mask = self.get_winin() as u8;
            for x in x1..x2 {
                self.window_mask[x] = mask;
            }
        }
    }

    /// Render a single scanline
    pub fn draw_scanline(&mut self) {
        let y = unsafe { (*self.gpu).get_vcount() } as usize;
        let base = y * PIXELS_PER_LINE;

        // Clear buffers
        for x in 0..PIXELS_PER_LINE {
            self.framebuffer[base + x] = 0xFF000000;
            self.front_framebuffer[base + x] = 0xFF000000;
            self.final_bg_priority[x] = 0xFF;
        }

        self.draw_backdrop();

        if self.dispcnt.display_win0 || self.dispcnt.display_win1 || self.dispcnt.obj_win_display {
            self.get_window_mask();
        } else {
            for i in 0..PIXELS_PER_LINE {
                self.window_mask[i] = 0xFF;
            }
        }

        // Draw BG layers by priority
        for priority in (0..4).rev() {
            for bg in (0..4).rev() {
                if (self.bgcnt[bg] & 0x3) as usize != priority {
                    continue;
                }

                if !self.dispcnt.bg_enabled(bg) {
                    continue;
                }

                if bg == 0 && self.engine_a && self.dispcnt.bg_3d {
                    unsafe {
                        (*self.gpu).draw_3d_scanline(
                            self.framebuffer.as_mut_ptr(),
                            self.final_bg_priority.as_mut_ptr(),
                            priority as u8,
                        );
                    }
                } else {
                    self.draw_bg_txt(bg);
                }
            }
        }

        if self.dispcnt.display_obj {
            self.draw_sprites();
        }

        self.apply_master_bright();
    }

    /// Draw text background layer
    pub fn draw_bg_txt(&mut self, index: usize) {
        let mut x_offset = self.bghofs[index] as u16;
        let y_offset = self.bgvofs[index].wrapping_add(unsafe { (*self.gpu).get_vcount() as u16 });

        let palette = unsafe { (*self.gpu).get_palette(self.engine_a) };

        let one_palette_mode = (self.bgcnt[index] & (1 << 7)) != 0;

        let mut screen_base: u32;
        let mut char_base: u32;

        if self.engine_a {
            screen_base = VRAM_BGA_START + (self.dispcnt.screen_base as u32 * 1024 * 64);
            char_base = VRAM_BGA_START + (self.dispcnt.char_base as u32 * 1024 * 64);
        } else {
            screen_base = VRAM_BGB_C;
            char_base = VRAM_BGB_C;
        }

        screen_base += ((self.bgcnt[index] >> 8) & 0x1F) as u32 * 1024 * 2;

        if (self.bgcnt[index] & 0x8000) != 0 {
            screen_base += ((y_offset & 0x1F8) as u32) * 8;
            if (self.bgcnt[index] & 0x4000) != 0 {
                screen_base += ((y_offset & 0x100) as u32) * 8;
            }
        } else {
            screen_base += ((y_offset & 0xF8) as u32) * 8;
        }

        char_base += ((self.bgcnt[index] >> 2) & 0xF) as u32 * 1024 * 16;

        let wide_x = if (self.bgcnt[index] & (1 << 14)) != 0 {
            0x100
        } else {
            0
        };

        let scanline = unsafe { (*self.gpu).get_vcount() as usize } * PIXELS_PER_LINE;

        if !one_palette_mode {
            // 16-color tiles
            let mut data: u32 = 0;
            let mut tile: u16 = 0;
            let mut tile_num: u16 = 0;
            let mut palette_id: u16 = 0;
            let mut x_flip: bool = false;
            let mut y_flip: bool = false;

            if (x_offset & 0x7) != 0 {
                let addr = screen_base
                    + (((x_offset & 0xF8) as u32) >> 2)
                    + (((x_offset & wide_x) as u32) << 3);

                tile = unsafe {
                    if self.engine_a {
                        (*self.gpu).read_bga::<u16>(addr)
                    } else {
                        (*self.gpu).read_bgb::<u16>(addr)
                    }
                };

                tile_num = tile & 0x3FF;
                x_flip = (tile & (1 << 10)) != 0;
                y_flip = (tile & (1 << 11)) != 0;
                palette_id = tile >> 12;

                let pixel_base = char_base
                    + (tile_num as u32 * 32)
                    + if y_flip {
                        (7 - (y_offset & 0x7)) as u32 * 4
                    } else {
                        (y_offset & 0x7) as u32 * 4
                    };

                data = unsafe {
                    if self.engine_a {
                        (*self.gpu).read_bga::<u32>(pixel_base)
                    } else {
                        (*self.gpu).read_bgb::<u32>(pixel_base)
                    }
                };
            }

            for pixel in 0..PIXELS_PER_LINE {
                if (x_offset & 0x7) == 0 {
                    let addr = screen_base
                        + (((x_offset & 0xF8) as u32) >> 2)
                        + (((x_offset & wide_x) as u32) << 3);

                    tile = unsafe {
                        if self.engine_a {
                            (*self.gpu).read_bga::<u16>(addr)
                        } else {
                            (*self.gpu).read_bgb::<u16>(addr)
                        }
                    };

                    tile_num = tile & 0x3FF;
                    x_flip = (tile & (1 << 10)) != 0;
                    y_flip = (tile & (1 << 11)) != 0;
                    palette_id = tile >> 12;

                    let pixel_base = char_base
                        + (tile_num as u32 * 32)
                        + if y_flip {
                            (7 - (y_offset & 0x7)) as u32 * 4
                        } else {
                            (y_offset & 0x7) as u32 * 4
                        };

                    data = unsafe {
                        if self.engine_a {
                            (*self.gpu).read_bga::<u32>(pixel_base)
                        } else {
                            (*self.gpu).read_bgb::<u32>(pixel_base)
                        }
                    };
                }

                let tile_x = if x_flip {
                    7 - (x_offset & 0x7)
                } else {
                    x_offset & 0x7
                };

                let color = ((data >> (tile_x * 4)) & 0xF) as u16;

                if color != 0 && (self.window_mask[pixel] & (1 << index)) != 0 {
                    let pal_color = palette[(palette_id as usize * 16) + color as usize];

                    let r = ((pal_color & 0x1F) << 3) as u32;
                    let g = (((pal_color >> 5) & 0x1F) << 3) as u32;
                    let b = (((pal_color >> 10) & 0x1F) << 3) as u32;

                    self.framebuffer[pixel + scanline] = 0xFF000000 | (r << 16) | (g << 8) | b;
                    self.final_bg_priority[pixel] = (self.bgcnt[index] & 0x3) as u8;
                }

                x_offset = x_offset.wrapping_add(1);
            }
        } else {
            // 256-color tiles
            let mut data: u64 = 0;
            let mut tile: u16 = 0;
            let mut tile_num: u16 = 0;
            let mut palette_id: u16 = 0;
            let mut x_flip: bool = false;
            let mut y_flip: bool = false;

            if (x_offset & 0x7) != 0 {
                let addr = screen_base
                    + (((x_offset & 0xF8) as u32) >> 2)
                    + (((x_offset & wide_x) as u32) << 3);

                tile = unsafe {
                    if self.engine_a {
                        (*self.gpu).read_bga::<u16>(addr)
                    } else {
                        (*self.gpu).read_bgb::<u16>(addr)
                    }
                };

                tile_num = tile & 0x3FF;
                x_flip = (tile & (1 << 10)) != 0;
                y_flip = (tile & (1 << 11)) != 0;
                palette_id = tile >> 12;

                let pixel_base = char_base
                    + (tile_num as u32 * 64)
                    + if y_flip {
                        (7 - (y_offset & 0x7)) as u32 * 8
                    } else {
                        (y_offset & 0x7) as u32 * 8
                    };

                data = unsafe {
                    if self.engine_a {
                        (*self.gpu).read_bga::<u64>(pixel_base)
                    } else {
                        (*self.gpu).read_bgb::<u64>(pixel_base)
                    }
                };
            }

            for pixel in 0..PIXELS_PER_LINE {
                if (x_offset & 0x7) == 0 {
                    let addr = screen_base
                        + (((x_offset & 0xF8) as u32) >> 2)
                        + (((x_offset & wide_x) as u32) << 3);

                    tile = unsafe {
                        if self.engine_a {
                            (*self.gpu).read_bga::<u16>(addr)
                        } else {
                            (*self.gpu).read_bgb::<u16>(addr)
                        }
                    };

                    tile_num = tile & 0x3FF;
                    x_flip = (tile & (1 << 10)) != 0;
                    y_flip = (tile & (1 << 11)) != 0;
                    palette_id = tile >> 12;

                    let pixel_base = char_base
                        + (tile_num as u32 * 64)
                        + if y_flip {
                            (7 - (y_offset & 0x7)) as u32 * 8
                        } else {
                            (y_offset & 0x7) as u32 * 8
                        };

                    data = unsafe {
                        if self.engine_a {
                            (*self.gpu).read_bga::<u64>(pixel_base)
                        } else {
                            (*self.gpu).read_bgb::<u64>(pixel_base)
                        }
                    };
                }

                let tile_x = if x_flip {
                    7 - (x_offset & 0x7)
                } else {
                    x_offset & 0x7
                };

                let mut color = ((data >> (tile_x * 8)) & 0xFF) as u16;

                if color != 0 && (self.window_mask[pixel] & (1 << index)) != 0 {
                    if self.dispcnt.bg_extended_palette {
                        let ext_base = index as u32 * 1024 * 8;
                        color = unsafe {
                            if self.engine_a {
                                (*self.gpu).read_extpal_bga(
                                    ext_base + (palette_id as u32 * 512) + (color as u32 * 2),
                                )
                            } else {
                                (*self.gpu).read_extpal_bgb(
                                    ext_base + (palette_id as u32 * 512) + (color as u32 * 2),
                                )
                            }
                        };
                    } else {
                        color = palette[color as usize];
                    }

                    let r = ((color & 0x1F) << 3) as u32;
                    let g = (((color >> 5) & 0x1F) << 3) as u32;
                    let b = (((color >> 10) & 0x1F) << 3) as u32;

                    self.framebuffer[pixel + scanline] = 0xFF000000 | (r << 16) | (g << 8) | b;
                    self.final_bg_priority[pixel] = (self.bgcnt[index] & 0x3) as u8;
                }

                x_offset = x_offset.wrapping_add(1);
            }
        }
    }
    /// Draw rotation/scale background (BG2/BG3)
    pub fn draw_bg_ext(&mut self, index: usize) {
        let scanline = unsafe { (*self.gpu).get_vcount() as usize } * PIXELS_PER_LINE;

        // 回転・拡大パラメータ
        let pa = self.bg_pa[index];
        let pb = self.bg_pb[index];
        let pc = self.bg_pc[index];
        let pd = self.bg_pd[index];

        // 背景マップのスクロール（固定小数点 8.8）
        let x0 = self.bg_x[index];
        let y0 = self.bg_y[index];

        // タイル / キャラクタベース
        let screen_base = if self.engine_a {
            VRAM_BGA_START + (((self.bgcnt[index] >> 8) & 0x1F) as u32 * 1024 * 2)
        } else {
            VRAM_BGB_C + (((self.bgcnt[index] >> 8) & 0x1F) as u32 * 1024 * 2)
        };

        let char_base = if self.engine_a {
            VRAM_BGA_START + (((self.bgcnt[index] >> 2) & 0xF) as u32 * 1024 * 16)
        } else {
            VRAM_BGB_C + (((self.bgcnt[index] >> 2) & 0xF) as u32 * 1024 * 16)
        };

        let one_palette_mode = (self.bgcnt[index] & (1 << 7)) != 0;

        let palette = unsafe { (*self.gpu).get_palette(self.engine_a) };

        // ピクセルごとに描画
        for x in 0..PIXELS_PER_LINE {
            // 背景座標（小数点あり）
            let mut bg_x = x0 + pa * (x as i32) + pb * (unsafe { (*self.gpu).get_vcount() as i32 });
            let mut bg_y = y0 + pc * (x as i32) + pd * (unsafe { (*self.gpu).get_vcount() as i32 });

            // マップ座標（8x8 タイル単位）
            let tile_x = ((bg_x >> 8) & 0x3F) as u32; // 32/64 タイル対応
            let tile_y = ((bg_y >> 8) & 0x3F) as u32;

            let map_addr = screen_base + tile_y * 32 * 2 + tile_x * 2;

            let tile: u16 = unsafe {
                if self.engine_a {
                    (*self.gpu).read_bga::<u16>(map_addr)
                } else {
                    (*self.gpu).read_bgb::<u16>(map_addr)
                }
            };

            let tile_num = tile & 0x3FF;
            let x_flip = (tile & (1 << 10)) != 0;
            let y_flip = (tile & (1 << 11)) != 0;
            let palette_id = tile >> 12;

            let px = if x_flip {
                7 - ((bg_x >> 8) & 0x7)
            } else {
                (bg_x >> 8) & 0x7
            };
            let py = if y_flip {
                7 - ((bg_y >> 8) & 0x7)
            } else {
                (bg_y >> 8) & 0x7
            };

            let pixel_base = char_base
                + (tile_num as u32 * if one_palette_mode { 32 } else { 64 })
                + (py as u32 * if one_palette_mode { 4 } else { 8 });

            let color: u16 = unsafe {
                if one_palette_mode {
                    let data = if self.engine_a {
                        (*self.gpu).read_bga::<u32>(pixel_base)
                    } else {
                        (*self.gpu).read_bgb::<u32>(pixel_base)
                    };
                    ((data >> (px * 4)) & 0xF) as u16
                } else {
                    let data = if self.engine_a {
                        (*self.gpu).read_bga::<u64>(pixel_base)
                    } else {
                        (*self.gpu).read_bgb::<u64>(pixel_base)
                    };
                    ((data >> (px * 8)) & 0xFF) as u16
                }
            };

            if color != 0 && (self.window_mask[x] & (1 << index)) != 0 {
                let pal_color = if one_palette_mode {
                    palette[(palette_id as usize * 16 + color as usize)]
                } else {
                    color
                };

                let r = ((pal_color & 0x1F) << 3) as u32;
                let g = (((pal_color >> 5) & 0x1F) << 3) as u32;
                let b = (((pal_color >> 10) & 0x1F) << 3) as u32;

                self.framebuffer[x + scanline] = 0xFF000000 | (r << 16) | (g << 8) | b;
                self.final_bg_priority[x] = (self.bgcnt[index] & 0x3) as u8;
            }
        }
    }

    /// Draw extended text (RotoScale BG text)
    pub fn draw_ext_text(&mut self, index: usize) {
        let scanline = unsafe { (*self.gpu).get_vcount() as usize } * PIXELS_PER_LINE;

        // extend/shrink
        let pa = self.bg_pa[index];
        let pb = self.bg_pb[index];
        let pc = self.bg_pc[index];
        let pd = self.bg_pd[index];

        let x0 = self.bg_x[index];
        let y0 = self.bg_y[index];

        // text map for page address
        let screen_base = if self.engine_a {
            VRAM_BGA_START + (((self.bgcnt[index] >> 8) & 0x1F) as u32 * 1024 * 2)
        } else {
            VRAM_BGB_C + (((self.bgcnt[index] >> 8) & 0x1F) as u32 * 1024 * 2)
        };

        let char_base = if self.engine_a {
            VRAM_BGA_START + (((self.bgcnt[index] >> 2) & 0xF) as u32 * 1024 * 16)
        } else {
            VRAM_BGB_C + (((self.bgcnt[index] >> 2) & 0xF) as u32 * 1024 * 16)
        };

        let one_palette_mode = (self.bgcnt[index] & (1 << 7)) != 0;
        let palette = unsafe { (*self.gpu).get_palette(self.engine_a) };

        for x in 0..PIXELS_PER_LINE {
            let mut bg_x = x0 + pa * (x as i32) + pb * (unsafe { (*self.gpu).get_vcount() as i32 });
            let mut bg_y = y0 + pc * (x as i32) + pd * (unsafe { (*self.gpu).get_vcount() as i32 });

            // title
            let tile_x = ((bg_x >> 8) & 0x1F) as u32;
            let tile_y = ((bg_y >> 8) & 0x1F) as u32;

            let map_addr = screen_base + tile_y * 32 * 2 + tile_x * 2;

            let tile: u16 = unsafe {
                if self.engine_a {
                    (*self.gpu).read_bga::<u16>(map_addr)
                } else {
                    (*self.gpu).read_bgb::<u16>(map_addr)
                }
            };

            let tile_num = tile & 0x3FF;
            let x_flip = (tile & (1 << 10)) != 0;
            let y_flip = (tile & (1 << 11)) != 0;
            let palette_id = tile >> 12;

            let px = if x_flip {
                7 - ((bg_x >> 8) & 0x7)
            } else {
                (bg_x >> 8) & 0x7
            };
            let py = if y_flip {
                7 - ((bg_y >> 8) & 0x7)
            } else {
                (bg_y >> 8) & 0x7
            };

            let pixel_base = char_base
                + (tile_num as u32 * if one_palette_mode { 32 } else { 64 })
                + (py as u32 * if one_palette_mode { 4 } else { 8 });

            let color: u16 = unsafe {
                if one_palette_mode {
                    let data = if self.engine_a {
                        (*self.gpu).read_bga::<u32>(pixel_base)
                    } else {
                        (*self.gpu).read_bgb::<u32>(pixel_base)
                    };
                    ((data >> (px * 4)) & 0xF) as u16
                } else {
                    let data = if self.engine_a {
                        (*self.gpu).read_bga::<u64>(pixel_base)
                    } else {
                        (*self.gpu).read_bgb::<u64>(pixel_base)
                    };
                    ((data >> (px * 8)) & 0xFF) as u16
                }
            };

            if color != 0 && (self.window_mask[x] & (1 << index)) != 0 {
                let pal_color = if one_palette_mode {
                    palette[(palette_id as usize * 16 + color as usize)]
                } else {
                    color
                };

                let r = ((pal_color & 0x1F) << 3) as u32;
                let g = (((pal_color >> 5) & 0x1F) << 3) as u32;
                let b = (((pal_color >> 10) & 0x1F) << 3) as u32;

                self.framebuffer[x + scanline] = 0xFF000000 | (r << 16) | (g << 8) | b;
                self.final_bg_priority[x] = (self.bgcnt[index] & 0x3) as u8;
            }
        }
        /// Draw a single rotation/scaling sprite
        pub fn draw_rotscale_sprite(&mut self, sprite_index: usize) {
            // Get the current scanline
            let scanline = unsafe { (*self.gpu).get_vcount() as usize } * PIXELS_PER_LINE;

            // Read sprite attributes
            let attr0 = self.oam[sprite_index * 4];
            let attr1 = self.oam[sprite_index * 4 + 1];
            let attr2 = self.oam[sprite_index * 4 + 2];
            let attr3 = self.oam[sprite_index * 4 + 3];

            // Check if sprite is visible on this scanline
            let y = attr0 & 0xFF;
            let height = (attr0 >> 14) & 0x3; // sprite size, simplified
            let scan_y = unsafe { (*self.gpu).get_vcount() as u16 };
            if scan_y < y || scan_y >= y + height * 8 {
                return; // not visible
            }

            // Extract rotation/scaling parameters
            let pa = self.obj_affine[sprite_index].pa;
            let pb = self.obj_affine[sprite_index].pb;
            let pc = self.obj_affine[sprite_index].pc;
            let pd = self.obj_affine[sprite_index].pd;

            let center_x = attr1 & 0xFF;
            let center_y = attr1 >> 8;

            let tile_index = attr2 & 0x3FF;
            let palette_id = (attr2 >> 12) & 0xF;
            let priority = (attr2 >> 10) & 0x3;

            let one_palette_mode = (attr2 & (1 << 13)) != 0;

            // Iterate through sprite width
            let width = match (attr1 >> 14) & 0x3 {
                0 => 8,
                1 => 16,
                2 => 32,
                3 => 64,
                _ => 8,
            };

            for x in 0..width {
                // Apply affine transformation
                let dx = pa * (x as i32) + pb * (scan_y as i32);
                let dy = pc * (x as i32) + pd * (scan_y as i32);

                let sprite_x = center_x as i32 + (dx >> 8);
                let sprite_y = center_y as i32 + (dy >> 8);

                if sprite_x < 0 || sprite_x >= PIXELS_PER_LINE as i32 {
                    continue;
                }

                // Calculate tile pixel
                let tile_x = (dx >> 8) & 7;
                let tile_y = (dy >> 8) & 7;

                let char_base =
                    VRAM_OBJ + (tile_index as u32 * if one_palette_mode { 32 } else { 64 });
                let pixel_base = char_base + (tile_y as u32 * if one_palette_mode { 4 } else { 8 });

                let color = unsafe {
                    if one_palette_mode {
                        let data = (*self.gpu).read_obj::<u32>(pixel_base);
                        ((data >> (tile_x * 4)) & 0xF) as u16
                    } else {
                        let data = (*self.gpu).read_obj::<u64>(pixel_base);
                        ((data >> (tile_x * 8)) & 0xFF) as u16
                    }
                };

                if color != 0 {
                    let pal_color = if one_palette_mode {
                        unsafe {
                            (*self.gpu).get_palette_obj()[palette_id as usize * 16 + color as usize]
                        }
                    } else {
                        color
                    };

                    let r = ((pal_color & 0x1F) << 3) as u32;
                    let g = (((pal_color >> 5) & 0x1F) << 3) as u32;
                    let b = (((pal_color >> 10) & 0x1F) << 3) as u32;

                    self.framebuffer[sprite_x as usize + scanline] =
                        0xFF000000 | (r << 16) | (g << 8) | b;
                    self.final_obj_priority[sprite_x as usize] = priority as u8;
                }
            }
        }
    }

    /// Draw all sprites for the current scanline
    pub fn draw_sprites(&mut self) {
        // Get current scanline
        let scan_y = unsafe { (*self.gpu).get_vcount() as u16 };

        // Clear priority buffer for this scanline
        for x in 0..PIXELS_PER_LINE {
            self.final_obj_priority[x] = 0xFF; // Max priority = empty
        }

        // Loop through all 128 possible sprites
        for i in 0..128 {
            let attr0 = self.oam[i * 4];

            // Extract sprite type
            let rot_scale_flag = (attr0 >> 8) & 0x1 != 0;
            let double_size = (attr0 >> 9) & 0x1 != 0;

            if rot_scale_flag {
                // Handle rotation/scaling sprite
                self.draw_rotscale_sprite(i);
            } else {
                // Handle normal sprite
                self.draw_normal_sprite(i);
            }
        }
    }

    /// Draw a normal (non-rotating) sprite
    fn draw_normal_sprite(&mut self, sprite_index: usize) {
        let attr0 = self.oam[sprite_index * 4];
        let attr1 = self.oam[sprite_index * 4 + 1];
        let attr2 = self.oam[sprite_index * 4 + 2];

        let y = attr0 & 0xFF;
        let x = attr1 & 0x1FF;

        let sprite_height = match (attr0 >> 14) & 0x3 {
            0 => 8,
            1 => 16,
            2 => 32,
            3 => 64,
            _ => 8,
        };

        let sprite_width = match (attr1 >> 14) & 0x3 {
            0 => 8,
            1 => 16,
            2 => 32,
            3 => 64,
            _ => 8,
        };

        let tile_index = attr2 & 0x3FF;
        let palette_id = (attr2 >> 12) & 0xF;
        let priority = (attr2 >> 10) & 0x3;
        let one_palette_mode = (attr2 & (1 << 13)) != 0;

        let scan_y = unsafe { (*self.gpu).get_vcount() as u16 };

        // Skip if this scanline is not within sprite vertical bounds
        if scan_y < y || scan_y >= y + sprite_height {
            return;
        }

        for px in 0..sprite_width {
            let framebuffer_x = x + px;
            if framebuffer_x >= PIXELS_PER_LINE as u16 {
                continue;
            }

            for py in 0..sprite_height {
                if y + py != scan_y {
                    continue; // Only process current scanline
                }

                // Compute tile pixel coordinates
                let tile_x = px % 8;
                let tile_y = py % 8;
                let char_base =
                    VRAM_OBJ + (tile_index as u32 * if one_palette_mode { 32 } else { 64 });
                let pixel_base = char_base + (tile_y as u32 * if one_palette_mode { 4 } else { 8 });

                let color = unsafe {
                    if one_palette_mode {
                        let data = (*self.gpu).read_obj::<u32>(pixel_base);
                        ((data >> (tile_x * 4)) & 0xF) as u16
                    } else {
                        let data = (*self.gpu).read_obj::<u64>(pixel_base);
                        ((data >> (tile_x * 8)) & 0xFF) as u16
                    }
                };

                if color == 0 {
                    continue; // Transparent pixel
                }

                let pal_color = if one_palette_mode {
                    unsafe {
                        (*self.gpu).get_palette_obj()[palette_id as usize * 16 + color as usize]
                    }
                } else {
                    color
                };

                let r = ((pal_color & 0x1F) << 3) as u32;
                let g = (((pal_color >> 5) & 0x1F) << 3) as u32;
                let b = (((pal_color >> 10) & 0x1F) << 3) as u32;

                self.framebuffer[framebuffer_x as usize + (scan_y as usize * PIXELS_PER_LINE)] =
                    0xFF000000 | (r << 16) | (g << 8) | b;
                self.final_obj_priority[framebuffer_x as usize] = priority as u8;
            }
        }
    }

    /// Render a single scanline, combining backgrounds and sprites
    pub fn render_scanline(&mut self) {
        let scan_y = unsafe { (*self.gpu).get_vcount() as u16 };

        // 1. Draw all backgrounds for this scanline
        for bg_id in 0..4 {
            let bg_enable = (unsafe { (*self.gpu).bg_control[bg_id] } & (1 << 7)) != 0;
            if bg_enable {
                self.draw_bg_scanline(bg_id as usize, scan_y);
            }
        }

        // 2. Draw sprites for this scanline
        self.draw_sprites();

        // 3. Merge backgrounds and sprites with priority
        for x in 0..PIXELS_PER_LINE {
            let sprite_priority = self.final_obj_priority[x];
            let sprite_pixel = self.framebuffer[x + (scan_y as usize * PIXELS_PER_LINE)];

            let mut final_pixel = sprite_pixel;
            let mut final_priority = sprite_priority;

            for bg_id in (0..4).rev() {
                let bg_enable = (unsafe { (*self.gpu).bg_control[bg_id] } & (1 << 7)) != 0;
                if !bg_enable {
                    continue;
                }

                let bg_priority = ((unsafe { (*self.gpu).bg_control[bg_id] } >> 2) & 0x3) as u8;
                let bg_pixel = self.bg_line_buffer[bg_id][x];

                // Merge: lower priority number = higher priority
                if bg_pixel != 0 && bg_priority <= final_priority {
                    final_pixel = bg_pixel;
                    final_priority = bg_priority;
                }
            }

            // Write final pixel to the screen framebuffer
            self.screen_framebuffer[x + (scan_y as usize * PIXELS_PER_LINE)] = final_pixel;
        }
    }

    /// Render the full frame by looping through all scanlines
    pub fn render_frame(&mut self) {
        for scanline in 0..SCREEN_HEIGHT {
            self.render_scanline();
        }
    }
}
