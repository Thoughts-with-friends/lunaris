use crate::gpu_root::Gpu;

impl Gpu {
    // text image
    pub fn read_teximage_u8(&self, address: u32) -> u8 {
        todo!()
    }

    pub fn read_teximage_u16(&self, address: u32) -> u16 {
        todo!()
    }

    // textpal
    pub fn read_texpal_u16(&self, address: u32) -> u16 {
        todo!()
    }

    pub fn read_texpal_u32(&self, address: u32) -> u32 {
        todo!()
    }

    // LCDC
    pub fn read_lcdc_u8(&self, address: u32) -> u8 {
        todo!()
    }

    pub fn read_lcdc_u16(&self, address: u32) -> u16 {
        todo!()
    }

    pub fn read_lcdc_u32(&self, address: u32) -> u32 {
        todo!()
    }

    // moved arm_rw.rs
    // pub fn read_oam<T>(&self, address: u32)
}
