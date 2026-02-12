use crate::gpu_root::Gpu;

impl Gpu {
    // ===== Write functions =====

    /// Write to palette A
    pub fn write_palette_a(&mut self, address: u32, value: u16) {
        let address = address as usize;
        if address < self.palette_upper.len() {
            self.palette_upper[address] = (value & 0xFF) as u8;
        }
        if address + 1 < self.palette_upper.len() {
            self.palette_upper[address + 1] = ((value >> 8) & 0xFF) as u8;
        }
    }

    /// Write to palette B
    pub fn write_palette_b(&mut self, address: u32, value: u16) {
        let address = address as usize;
        if address < self.palette_lower.len() {
            self.palette_lower[address] = (value & 0xFF) as u8;
        }
        if address + 1 < self.palette_lower.len() {
            self.palette_lower[address + 1] = ((value >> 8) & 0xFF) as u8;
        }
    }

    pub fn write_bga(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    pub fn write_bgb(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    pub fn write_obja(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    pub fn write_objb(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    pub fn write_lcdc(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    pub fn write_oam(&mut self, address: u32, halfword: u16) {
        todo!()
    }
}
