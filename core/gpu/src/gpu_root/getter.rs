use crate::gpu_root::Gpu;

impl Gpu {
    pub fn get_palette(&mut self, engine_a: bool) -> &mut [u16] {
        todo!()
    }

    // TODO: id to enum?
    /// Get VRAM bank by ID.
    /// # Panics
    /// Returns empty slice if ID is 0..=3 outside valid range.
    pub fn get_vram_block(&self, id: i32) -> &[u16] {
        let bytes = match id {
            0 => &self.vram_a,
            1 => &self.vram_b,
            2 => &self.vram_c,
            3 => &self.vram_d,
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Invalid VRAM bank ID: {id}");
                panic!("Invalid VRAM bank ID: {id}");
            }
        };
        assert_eq!(bytes.len() % 2, 0, "u8 vector length must be even");
        unsafe { core::slice::from_raw_parts(bytes.as_ptr() as *mut u16, bytes.len() / 2) }
    }

    // TODO: id to enum?
    /// Get VRAM bank by ID.
    /// # Panics
    /// Returns empty slice if ID is 0..=3 outside valid range.
    pub fn get_vram_block_mut(&mut self, id: i32) -> &mut [u16] {
        let bytes = match id {
            0 => &mut self.vram_a,
            1 => &mut self.vram_b,
            2 => &mut self.vram_c,
            3 => &mut self.vram_d,
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Invalid VRAM bank ID: {id}");
                panic!("Invalid VRAM bank ID: {id}");
            }
        };
        assert_eq!(bytes.len() % 2, 0, "u8 vector length must be even");
        unsafe { core::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut u16, bytes.len() / 2) }
    }

    // TODO: id to enum?
    /// Get VRAM bank by ID.
    /// # Panics
    /// Returns empty slice if ID is 0..=3 outside valid range.
    pub fn get_movable_vram_block(&mut self, src_id: i32, dest_id: i32) -> (&[u16], &mut [u16]) {
        let src_bytes = match src_id {
            0 => &self.vram_a,
            1 => &self.vram_b,
            2 => &self.vram_c,
            3 => &self.vram_d,
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Invalid VRAM bank ID: {src_id}");
                panic!("Invalid VRAM bank ID: {src_id}");
            }
        };
        let dest_bytes = match dest_id {
            0 => &self.vram_a,
            1 => &self.vram_b,
            2 => &self.vram_c,
            3 => &self.vram_d,
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Invalid VRAM bank ID: {dest_id}");
                panic!("Invalid id VRAM bank ID: {dest_id}");
            }
        };
        assert!(
            (src_bytes.len() % 2) == 0 && (dest_bytes.len() % 2) == 0,
            "u8 vector length must be even"
        );
        unsafe {
            (
                core::slice::from_raw_parts(src_bytes.as_ptr() as *mut u16, src_bytes.len() / 2),
                core::slice::from_raw_parts_mut(
                    dest_bytes.as_ptr() as *mut u16,
                    dest_bytes.len() / 2,
                ),
            )
        }
    }

    pub fn get_dispcnt_a(&self) -> u32 {
        self.engine_upper.get_dispcnt()
    }

    pub fn get_dispcnt_b(&self) -> u32 {
        self.engine_lower.get_dispcnt()
    }

    /// Get DISPSTAT7 register value
    pub fn get_dispstat7(&self) -> u16 {
        self.display_status_arm7.get()
    }

    /// Get DISPSTAT9 register value
    pub fn get_dispstat9(&self) -> u16 {
        self.display_status_arm9.get()
    }

    /// Get BGCNT
    pub fn get_bgcnt_a(&self, index: usize) -> u16 {
        self.engine_upper.get_bgcnt(index)
    }

    pub fn get_bgcnt_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgcnt(index)
    }

    /// Get VCOUNT register value
    pub fn get_vcount(&self) -> u16 {
        self.vertical_count
    }

    /// Get BGH
    pub fn get_bghofs_a(&self, index: usize) -> u16 {
        self.engine_upper.get_bgvofs(index)
    }

    pub fn get_bgvofs_a(&self, index: usize) -> u16 {
        self.engine_upper.get_bgvofs(index)
    }

    pub fn get_bghofs_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgcnt(index)
    }

    pub fn get_bgvofs_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgvofs(index)
    }

    pub fn get_win0v_a(&self) -> u16 {
        todo!()
    }

    pub fn get_win1v_a(&self) -> u16 {
        todo!()
    }

    pub fn get_win0v_b(&self) -> u16 {
        todo!()
    }

    pub fn get_win1v_b(&self) -> u16 {
        todo!()
    }

    pub fn get_winin_a(&self) -> u16 {
        todo!()
    }

    pub fn get_winin_b(&self) -> u16 {
        todo!()
    }

    pub fn get_winout_a(&self) -> u16 {
        todo!()
    }

    pub fn get_winout_b(&self) -> u16 {
        todo!()
    }

    pub fn get_bldcnt_a(&self) -> u16 {
        todo!()
    }

    pub fn get_bldcnt_b(&self) -> u16 {
        todo!()
    }

    pub fn get_bldalpha_a(&self) -> u16 {
        todo!()
    }

    pub fn get_bldalpha_b(&self) -> u16 {
        todo!()
    }

    pub fn get_disp3dcnt(&self) -> u16 {
        todo!()
    }

    pub fn get_master_bright_a(&self) -> u16 {
        todo!()
    }

    pub fn get_master_bright_b(&self) -> u16 {
        todo!()
    }

    /// Replace uint32_t get_DISPCAPCNT();
    pub fn get_dispcapcnt_a(&self) -> u32 {
        let is_engine_a = true;
        self.engine_upper.get_dispcapcnt(is_engine_a)
    }

    pub fn get_vramstat(&self) -> u8 {
        let mut reg: u8 = 0;
        reg |= (self.vramcnt_c.enabled && self.vramcnt_c.mst == 2) as u8;
        reg |= ((self.vramcnt_d.enabled && self.vramcnt_d.mst == 2) as u8) << 1;
        reg
    }

    /// Get VRAM bank configuration A
    pub fn get_vramcnt_a(&self) -> u8 {
        ((self.vramcnt_a.mst & 0x7) as u8) | (if self.vramcnt_a.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration B
    pub fn get_vramcnt_b(&self) -> u8 {
        ((self.vramcnt_b.mst & 0x7) as u8) | (if self.vramcnt_b.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration C
    pub fn get_vramcnt_c(&self) -> u8 {
        ((self.vramcnt_c.mst & 0x7) as u8) | (if self.vramcnt_c.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration D
    pub fn get_vramcnt_d(&self) -> u8 {
        ((self.vramcnt_d.mst & 0x7) as u8) | (if self.vramcnt_d.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration E
    pub fn get_vramcnt_e(&self) -> u8 {
        ((self.vramcnt_e.mst & 0x7) as u8) | (if self.vramcnt_e.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration F
    pub fn get_vramcnt_f(&self) -> u8 {
        ((self.vramcnt_f.mst & 0x7) as u8) | (if self.vramcnt_f.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration G
    pub fn get_vramcnt_g(&self) -> u8 {
        ((self.vramcnt_g.mst & 0x7) as u8) | (if self.vramcnt_g.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration H
    pub fn get_vramcnt_h(&self) -> u8 {
        ((self.vramcnt_h.mst & 0x7) as u8) | (if self.vramcnt_h.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration I
    pub fn get_vramcnt_i(&self) -> u8 {
        ((self.vramcnt_i.mst & 0x7) as u8) | (if self.vramcnt_i.enabled { 0x80 } else { 0 })
    }

    /// Get POWCNT1 register value
    pub fn get_powcnt1(&self) -> u16 {
        self.power_control_reg.get()
    }

    pub fn get_gxstat(&self) -> u32 {
        todo!()
    }

    pub fn get_vert_count(&self) -> u16 {
        todo!()
    }

    pub fn get_poly_count(&self) -> u16 {
        todo!()
    }
}
