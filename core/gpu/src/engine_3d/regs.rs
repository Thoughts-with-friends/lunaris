// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! 3D engine registers (0x04000060, 0x04000320-0x040003BF,
//! 0x04000600-0x040006A3) and the VBlank buffer swap.
//!
//! Follows melonDS `GPU3D::Read32/Write*` and `GPU3D::VBlank`.

use crate::engine_3d::state::Gpu3D;

impl Gpu3D {
    /// Whether `address` is a 3D register handled by [`Self::write_reg`] /
    /// [`Self::read_reg`]. (GXSTAT, GXFIFO and the command ports are
    /// handled by the emulator, which drives the FIFO.)
    pub const fn is_3d_reg(address: u32) -> bool {
        matches!(address,
            // DS 3D Status: https://problemkaputt.de/gbatek.htm#ds3dstatus
            0x0400_0320..=0x0400_03BF // RDLINES_COUNT - Rendered Line Count Register .. TOON_TABLE - Toon Table, 32 colors
            | 0x0400_0604..=0x0400_0607 // RAM_COUNT - Polygon List & Vertex RAM Count Register
            // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
            | 0x0400_0610..=0x0400_0611 // DISP_1DOT_DEPTH - 1-Dot Polygon Display Boundary Depth
            // DS 3D I/O Map: https://problemkaputt.de/gbatek.htm#ds3diomap
            | 0x0400_0620..=0x0400_06A3) // POS_RESULT - Position Test Results .. VECMTX_RESULT - Read Current Directional Vector Matrix
    }

    /// Writes the bytes of `value` selected by `mask` to the word at
    /// `address & !3`.
    pub fn write_reg(&mut self, address: u32, value: u32, mask: u32) {
        let word = address & !3;
        let merge = |old: u32| (old & !mask) | (value & mask);
        match word {
            0x0400_0330..=0x0400_033F => {
                // EDGE_COLOR - Edge Colors 0..7
                let i = ((word - 0x0400_0330) >> 1) as usize; // EDGE_COLOR - Edge Colors 0..7
                let old = u32::from(self.edge_table[i]) | (u32::from(self.edge_table[i + 1]) << 16); // two 16-bit table entries per word
                let v = merge(old);
                self.edge_table[i] = v as u16;
                self.edge_table[i + 1] = (v >> 16) as u16;
            }
            // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
            0x0400_0340 => self.alpha_ref_val = merge(self.alpha_ref_val) & 0x1F, // ALPHA_TEST_REF - Alpha-Test Comparision Value
            // DS 3D Rear-Plane: https://problemkaputt.de/gbatek.htm#ds3drearplane
            0x0400_0350 => self.clear_attr1 = merge(self.clear_attr1), // CLEAR_COLOR - Clear Color Attribute Register
            0x0400_0354 => self.clear_attr2 = merge(self.clear_attr2), // CLEAR_DEPTH - Clear Depth Register
            // DS 3D Toon, Edge, Fog, Alpha-Blending, Anti-Aliasing: https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing
            0x0400_0358 => self.fog_color = merge(self.fog_color), // FOG_COLOR - Fog Color
            0x0400_035C => self.fog_offset = merge(self.fog_offset) & 0x7FFF, // FOG_OFFSET - Fog Depth Offset
            // DS 3D I/O Map: https://problemkaputt.de/gbatek.htm#ds3diomap
            0x0400_0360..=0x0400_037F => {
                // FOG_TABLE - Fog Density Table, 32 entries
                let i = (word - 0x0400_0360) as usize; // FOG_TABLE - Fog Density Table, 32 entries
                let old =
                    u32::from_le_bytes(self.fog_density[i..i + 4].try_into().unwrap_or_default());
                let v = merge(old).to_le_bytes();
                for (d, b) in self.fog_density[i..i + 4].iter_mut().zip(v) {
                    *d = b & 0x7F;
                }
            }
            0x0400_0380..=0x0400_03BF => {
                // TOON_TABLE - Toon Table, 32 colors
                let i = ((word - 0x0400_0380) >> 1) as usize; // TOON_TABLE - Toon Table, 32 colors
                let old = u32::from(self.toon_table[i]) | (u32::from(self.toon_table[i + 1]) << 16); // two 16-bit table entries per word
                let v = merge(old);
                self.toon_table[i] = v as u16;
                self.toon_table[i + 1] = (v >> 16) as u16;
            }
            // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
            0x0400_0610 if mask & 0xFFFF != 0 => {
                // DISP_1DOT_DEPTH - 1-Dot Polygon Display Boundary Depth
                let v = merge(0) & 0x7FFF;
                self.zero_dot_w_limit = (v * 0x200 + 0x1FF) as i32; // W limit = value * 200h + 1FFh
            }
            _ => {}
        }
    }

    /// 32-bit read of the 3D register word at `address & !3` (0 if
    /// unmapped / write-only).
    pub fn read_reg(&self, address: u32) -> u32 {
        let word = address & !3;
        match word {
            // RDLINES_COUNT: the renderer never runs out of time.
            // DS 3D Status: https://problemkaputt.de/gbatek.htm#ds3dstatus
            0x0400_0320 => 46, // RDLINES_COUNT - Rendered Line Count Register
            0x0400_0604 => self.num_polygons as u32 | ((self.num_vertices as u32) << 16), // RAM_COUNT - Polygon List & Vertex RAM Count Register
            // DS 3D I/O Map: https://problemkaputt.de/gbatek.htm#ds3diomap
            0x0400_0620..=0x0400_062F => self.pos_test_result[((word >> 2) & 3) as usize] as u32, // POS_RESULT - Position Test Results
            0x0400_0630..=0x0400_0637 => {
                // VEC_RESULT - Vector Test Results
                let i = ((word - 0x0400_0630) >> 1) as usize; // VEC_RESULT - Vector Test Results
                let lo = self.vec_test_result[i] as u16;
                let hi = self.vec_test_result.get(i + 1).map_or(0, |&v| v as u16);
                u32::from(lo) | (u32::from(hi) << 16) // two 16-bit results per word
            }
            0x0400_0640..=0x0400_067F => {
                // CLIPMTX_RESULT - Read Current Clip Coordinates Matrix
                self.current_clip_matrix()[((word >> 2) & 0xF) as usize] as u32 // CLIPMTX_RESULT: matrix element index (4-byte steps)
            }
            0x0400_0680..=0x0400_06A3 => {
                // VECMTX_RESULT - Read Current Directional Vector Matrix
                let i = ((word - 0x0400_0680) >> 2) as usize; // VECMTX_RESULT - Read Current Directional Vector Matrix
                self.vec_matrix[(i / 3) * 4 + i % 3] as u32
            }
            _ => 0,
        }
    }

    // ===== DISP3DCNT / GXSTAT =====

    /// DISP3DCNT, including the underflow/overflow status bits 12-13.
    pub fn get_disp3dcnt(&self) -> u16 {
        self.disp_cnt as u16
    }

    /// DISP3DCNT write; writing 1 to bits 12-13 acknowledges them.
    pub fn set_disp3dcnt(&mut self, value: u16) {
        let v = u32::from(value);
        self.disp_cnt = (v & 0x4FFF) | (self.disp_cnt & 0x3000);
        // Bits 12/13 are acknowledged by writing 1.
        self.disp_cnt &= !(v & 0x3000);
    }

    /// GXSTAT without the FIFO bits, which the emulator adds.
    pub fn get_gxstat(&self) -> u32 {
        let fifo = self.gxfifo.len() as u32;
        let busy = !self.gxfifo.is_empty()
            || !self.gxpipe.is_empty()
            || self.swap_buffers
            || self.cycles > 0;
        let test_busy = self
            .gxpipe
            .front()
            // DS 3D Status: https://problemkaputt.de/gbatek.htm#ds3dstatus
            .is_some_and(|c| (0x70..=0x72).contains(&c.command)); // BOX_TEST / POS_TEST / VEC_TEST still pending
        let stack_busy =
            self.gxpipe.front().is_some_and(|c| c.command == 0x11 || c.command == 0x12); // MTX_PUSH / MTX_POP still pending
        (self.gxstat & 0xC000_8002) // keep test result (bit 1), stack error (bit 15), FIFO IRQ mode (bits 30-31)
            | u32::from(test_busy)
            | ((self.pos_stack_ptr & 0x1F) << 8) // position/vector matrix stack level (bits 8-12)
            // DS 3D Status: https://problemkaputt.de/gbatek.htm#ds3dstatus
            | ((self.proj_stack_ptr & 1) << 13) // GXSTAT bit 13: projection matrix stack level
            | (u32::from(stack_busy) << 14) // GXSTAT bit 14: matrix stack busy
            | (fifo.min(256) << 16) // GXSTAT bit 16: command FIFO entries (bits 16-24)
            | (u32::from(fifo < 128) << 25) // GXSTAT bit 25: FIFO less than half full
            | (u32::from(fifo == 0) << 26) // GXSTAT bit 26: FIFO empty
            | (u32::from(busy) << 27) // GXSTAT bit 27: geometry engine busy
    }

    /// GXSTAT write: bit 15 acknowledges a matrix stack error, bits 30-31
    /// select the FIFO IRQ mode.
    pub fn write_gxstat(&mut self, value: u32, mask: u32) {
        let value = value & mask;
        if value & 0x8000 != 0 {
            // bit 15: writing 1 acknowledges the matrix stack overflow error
            self.gxstat &= !0x8000; // bit 15: matrix stack overflow/underflow error
            self.proj_stack_ptr = 0;
            self.tex_stack_ptr = 0;
        }
        if mask & 0xC000_0000 != 0 {
            // bits 30-31: GXFIFO IRQ condition
            self.gxstat = (self.gxstat & 0x3FFF_FFFF) | (value & 0xC000_0000); // bits 30-31: GXFIFO IRQ condition
        }
    }

    /// FIFO IRQ mode (GXSTAT bits 30-31).
    pub const fn gxfifo_irq_mode(&self) -> u32 {
        self.gxstat >> 30
    }

    /// Vertices in the current polygon RAM bank.
    pub fn get_vert_count(&self) -> u16 {
        self.num_vertices as u16
    }

    /// Polygons in the current polygon RAM bank.
    pub fn get_poly_count(&self) -> u16 {
        self.num_polygons as u16
    }

    /// VBlank (melonDS `GPU3D::VBlank`): on a pending SWAP_BUFFERS the
    /// polygon list becomes the render list; the rendering registers are
    /// latched every frame.
    ///
    /// ```text
    /// from: ds_emu gpu_handle_event() at line 192 (VBlank start)
    ///   |
    ///   v
    /// end_of_frame()
    ///   1. SWAP_BUFFERS pending:
    ///        render list = opaque polygons, then translucent ones,
    ///        Y-sorted (translucent too unless manual sort was asked)
    ///        render bank = current bank; flip to the other bank, reset
    ///        polygon/vertex counts; GX commands may run again
    ///   2. latch DISP3DCNT, clear color/depth, fog, edge and toon tables
    ///      into `render` (dirty if anything changed)
    ///   |
    ///   v
    /// to:   render_3d_frame() at line 215
    /// ```
    pub fn end_of_frame(&mut self) {
        if self.swap_buffers {
            let bank = self.cur_bank;
            let polys = &self.polygon_ram[bank][..self.num_polygons];
            self.render_polygons.clear();
            self.render_polygons.extend((0..polys.len()).filter(|&i| !polys[i].translucent));
            self.render_polygons.extend((0..polys.len()).filter(|&i| polys[i].translucent));
            // Y-sorting (stable): all polygons, or only the opaque ones
            // when SWAP_BUFFERS bit 0 (manual translucent sort) is set.
            let sorted =
                if self.flush_attributes & 1 != 0 { self.num_opaque_polygons } else { polys.len() };
            self.render_polygons[..sorted].sort_by_key(|&i| polys[i].sort_key);
            self.render_bank = bank;
            self.rend_poly_count = self.num_polygons as i32;
            self.rend_vert_count = self.num_vertices as i32;

            self.cur_bank ^= 1;
            self.num_vertices = 0;
            self.num_polygons = 0;
            self.num_opaque_polygons = 0;
            self.last_strip_polygon = None;
            self.swap_buffers = false;
            self.render_dirty = true;
        }

        let old = self.render.clone();
        let r = &mut self.render;
        r.disp_cnt = self.disp_cnt;
        // DS 3D Display Control: https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol
        r.alpha_ref = if self.disp_cnt & (1 << 2) != 0 {
            // DISP3DCNT bit 2: alpha-test enable
            self.alpha_ref_val
        } else {
            0
        };
        r.clear_attr1 = self.clear_attr1;
        r.clear_attr2 = self.clear_attr2;
        r.fog_color = self.fog_color;
        // DS 3D Toon, Edge, Fog, Alpha-Blending, Anti-Aliasing: https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing
        r.fog_offset = self.fog_offset * 0x200; // FOG_OFFSET in depth-buffer units
        r.fog_shift = (self.disp_cnt >> 8) & 0xF; // DISP3DCNT bits 8-11: fog shift
        r.fog_density[0] = self.fog_density[0];
        r.fog_density[1..33].copy_from_slice(&self.fog_density);
        r.fog_density[33] = self.fog_density[31];
        r.edge_table = self.edge_table;
        r.toon_table = self.toon_table;
        if *r != old {
            self.render_dirty = true;
        }
    }
}
