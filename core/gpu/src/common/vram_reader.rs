use crate::common::{
    Gpu, VRAM_A_SIZE, VRAM_B_SIZE, VRAM_C_SIZE, VRAM_D_SIZE, VRAM_E_SIZE, VRAM_F_SIZE, VRAM_G_SIZE,
    VRAM_H_SIZE, VRAM_I_SIZE,
};
use mem_const::*;

impl Gpu {
    pub fn read_bga_u8(&self, address: u32) -> u8 {
        let mut reg: u8 = 0;

        // VRAM A
        if self.vramcnt_a.enabled && self.vramcnt_a.mst == 1 {
            let base = VRAM_BGA_START + (self.vramcnt_a.offset * 0x20000);
            if addr_in_range(address, base, VRAM_A_SIZE as u32) {
                if let Some(&val) = self.vram_a.get((address & VRAM_A_MASK) as usize) {
                    reg |= val;
                }
            }
        }

        // VRAM B
        if self.vramcnt_b.enabled && self.vramcnt_b.mst == 1 {
            let base = VRAM_BGA_START + (self.vramcnt_b.offset * 0x20000);
            if addr_in_range(address, base, VRAM_B_SIZE as u32) {
                if let Some(&val) = self.vram_b.get((address & VRAM_B_MASK) as usize) {
                    reg |= val;
                }
            }
        }

        // VRAM C
        if self.vramcnt_c.enabled && self.vramcnt_c.mst == 1 {
            let base = VRAM_BGA_START + (self.vramcnt_c.offset * 0x20000);
            if addr_in_range(address, base, VRAM_C_SIZE as u32) {
                if let Some(&val) = self.vram_c.get((address & VRAM_C_MASK) as usize) {
                    reg |= val;
                }
            }
        }

        // VRAM D
        if self.vramcnt_d.enabled && self.vramcnt_d.mst == 1 {
            let base = VRAM_BGA_START + (self.vramcnt_d.offset * 0x20000);
            if addr_in_range(address, base, VRAM_D_SIZE as u32) {
                if let Some(&val) = self.vram_d.get((address & VRAM_D_MASK) as usize) {
                    reg |= val;
                }
            }
        }

        // VRAM E
        if self.vramcnt_e.enabled && self.vramcnt_e.mst == 1 {
            if addr_in_range(address, VRAM_BGA_START, VRAM_E_SIZE as u32) {
                if let Some(&val) = self.vram_e.get((address & VRAM_E_MASK) as usize) {
                    reg |= val;
                }
            }
        }

        // VRAM F
        if self.vramcnt_f.enabled && self.vramcnt_f.mst == 1 {
            let f_offset =
                (self.vramcnt_f.offset & 0x1) * 0x4000 + (self.vramcnt_f.offset & 0x2) * 0x10000;
            if addr_in_range(address, VRAM_BGA_START, f_offset) {
                if let Some(&val) = self.vram_f.get((address & VRAM_F_MASK) as usize) {
                    reg |= val;
                }
            }
        }

        // VRAM G
        if self.vramcnt_g.enabled && self.vramcnt_g.mst == 1 {
            let g_offset =
                (self.vramcnt_g.offset & 0x1) * 0x4000 + (self.vramcnt_g.offset & 0x2) * 0x10000;
            if addr_in_range(address, VRAM_BGA_START, g_offset) {
                if let Some(&val) = self.vram_g.get((address & VRAM_G_MASK) as usize) {
                    reg |= val;
                }
            }
        }

        reg
    }

    pub fn read_bga_u16(&self, address: u32) -> u16 {
        let mut bytes = [0u8; 2];
        bytes[0] = self.read_bga_u8(address);
        bytes[1] = self.read_bga_u8(address + 1);
        u16::from_le_bytes(bytes)
    }

    pub fn read_bga_u32(&self, address: u32) -> u32 {
        let mut bytes = [0u8; 4];
        for i in 0..4 {
            bytes[i] = self.read_bga_u8(address + i as u32);
        }
        u32::from_le_bytes(bytes)
    }

    pub fn read_bga_u64(&self, address: u32) -> u64 {
        let mut bytes = [0u8; 8];
        for i in 0..8 {
            bytes[i] = self.read_bga_u8(address + i as u32);
        }
        u64::from_le_bytes(bytes)
    }

    pub fn read_bgb_u8(&self, address: u32) -> u8 {
        let mut value: u8 = 0;

        // VRAM C (MST == 4)
        if addr_in_range(address, VRAM_BGB_C, VRAM_C_SIZE as u32) && self.vramcnt_c.mst == 4 {
            if self.vramcnt_c.enabled {
                if let Some(&v) = self.vram_c.get((address & VRAM_C_MASK) as usize) {
                    value |= v;
                }
            }
        }

        // VRAM H (MST == 1)
        if addr_in_range(address, VRAM_BGB_H, VRAM_H_SIZE as u32) && self.vramcnt_h.mst == 1 {
            if self.vramcnt_h.enabled {
                if let Some(&v) = self.vram_h.get((address & VRAM_H_MASK) as usize) {
                    value |= v;
                }
            }
        }

        // VRAM I (MST == 1)
        if addr_in_range(address, VRAM_BGB_I, VRAM_I_SIZE as u32) && self.vramcnt_i.mst == 1 {
            if self.vramcnt_i.enabled {
                if let Some(&v) = self.vram_i.get((address & VRAM_I_MASK) as usize) {
                    value |= v;
                }
            }
        }

        value
    }

    pub fn read_bgb_u16(&self, address: u32) -> u16 {
        u16::from_le_bytes([self.read_bgb_u8(address), self.read_bgb_u8(address + 1)])
    }

    pub fn read_bgb_u32(&self, address: u32) -> u32 {
        let mut bytes = [0u8; 4];
        for i in 0..4 {
            bytes[i] = self.read_bgb_u8(address + i as u32);
        }
        u32::from_le_bytes(bytes)
    }

    pub fn read_bgb_u64(&self, address: u32) -> u64 {
        let mut bytes = [0u8; 8];
        for i in 0..8 {
            bytes[i] = self.read_bgb_u8(address + i as u32);
        }
        u64::from_le_bytes(bytes)
    }

    pub fn read_obja_u8(&self, address: u32) -> u8 {
        let mut value: u8 = 0;

        // VRAM A (MST == 2)
        if addr_in_range(
            address,
            VRAM_OBJA_START + ((self.vramcnt_a.offset & 0x1) * 0x20000),
            VRAM_A_SIZE as u32,
        ) && self.vramcnt_a.mst == 2
        {
            if self.vramcnt_a.enabled {
                if let Some(&v) = self.vram_a.get((address & VRAM_A_MASK) as usize) {
                    value |= v;
                }
            }
        }

        // VRAM B (MST == 2)
        if addr_in_range(
            address,
            VRAM_OBJA_START + ((self.vramcnt_b.offset & 0x1) * 0x20000),
            VRAM_B_SIZE as u32,
        ) && self.vramcnt_b.mst == 2
        {
            if self.vramcnt_b.enabled {
                if let Some(&v) = self.vram_b.get((address & VRAM_B_MASK) as usize) {
                    value |= v;
                }
            }
        }

        // VRAM E (MST == 2)
        if addr_in_range(address, VRAM_OBJA_START, VRAM_E_SIZE as u32) && self.vramcnt_e.mst == 2 {
            if self.vramcnt_e.enabled {
                if let Some(&v) = self.vram_e.get((address & VRAM_E_MASK) as usize) {
                    value |= v;
                }
            }
        }

        // VRAM F (MST == 2)
        let f_offset =
            (self.vramcnt_f.offset & 0x1) * 0x4000 + (self.vramcnt_f.offset & 0x2) * 0x10000;
        if addr_in_range(address, VRAM_OBJA_START, f_offset) && self.vramcnt_f.mst == 2 {
            if self.vramcnt_f.enabled {
                if let Some(&v) = self.vram_f.get((address & VRAM_F_MASK) as usize) {
                    value |= v;
                }
            }
        }

        // VRAM G (MST == 2)
        let g_offset =
            (self.vramcnt_g.offset & 0x1) * 0x4000 + (self.vramcnt_g.offset & 0x2) * 0x10000;
        if addr_in_range(address, VRAM_OBJA_START, g_offset) && self.vramcnt_g.mst == 2 {
            if self.vramcnt_g.enabled {
                if let Some(&v) = self.vram_g.get((address & VRAM_G_MASK) as usize) {
                    value |= v;
                }
            }
        }

        value
    }

    pub fn read_obja_u16(&self, address: u32) -> u16 {
        u16::from_le_bytes([self.read_obja_u8(address), self.read_obja_u8(address + 1)])
    }

    pub fn read_obja_u32(&self, address: u32) -> u32 {
        let mut bytes = [0u8; 4];
        for i in 0..4 {
            bytes[i] = self.read_obja_u8(address + i as u32);
        }
        u32::from_le_bytes(bytes)
    }

    pub fn read_obja_u64(&self, address: u32) -> u64 {
        let mut bytes = [0u8; 8];
        for i in 0..8 {
            bytes[i] = self.read_obja_u8(address + i as u32);
        }
        u64::from_le_bytes(bytes)
    }

    pub fn read_objb_u8(&self, address: u32) -> u8 {
        let (in_range_d, in_range_i) = (
            addr_in_range(address, VRAM_OBJB_START, VRAM_D_SIZE as u32) && self.vramcnt_d.mst == 4,
            addr_in_range(address, VRAM_OBJB_START, VRAM_I_SIZE as u32) && self.vramcnt_i.mst == 2,
        );

        let (d_enabled, i_enabled) = (
            in_range_d && self.vramcnt_d.enabled,
            in_range_i && self.vramcnt_i.enabled,
        );

        let mut reg = 0;
        if d_enabled {
            if let Some(&v) = self.vram_d.get((address & VRAM_D_MASK) as usize) {
                reg |= v;
            }
        }
        if i_enabled {
            if let Some(&v) = self.vram_i.get((address & VRAM_D_MASK) as usize) {
                reg |= v;
            }
        }
        reg
    }

    pub fn read_objb_u16(&self, address: u32) -> u16 {
        u16::from_le_bytes([self.read_objb_u8(address), self.read_objb_u8(address + 1)])
    }

    pub fn read_objb_u32(&self, address: u32) -> u32 {
        let mut bytes = [0u8; 4];
        for i in 0..4 {
            bytes[i] = self.read_objb_u8(address + i as u32);
        }
        u32::from_le_bytes(bytes)
    }

    pub fn read_objb_u64(&self, address: u32) -> u64 {
        let mut bytes = [0u8; 8];
        for i in 0..8 {
            bytes[i] = self.read_objb_u8(address + i as u32);
        }
        u64::from_le_bytes(bytes)
    }

    pub fn read_extpal_bga_u16(&self, address: u32) -> u16 {
        let mut value: u16 = 0;

        // VRAM E (MST == 4)
        if self.vramcnt_e.enabled {
            if addr_in_range(address, 0, (VRAM_E_SIZE / 2) as u32) && self.vramcnt_e.mst == 4 {
                if let (Some(&lo), Some(&hi)) = (
                    self.vram_e.get((address & VRAM_E_MASK) as usize),
                    self.vram_e.get(((address & VRAM_E_MASK) + 1) as usize),
                ) {
                    value |= u16::from_le_bytes([lo, hi]);
                }
            }
        }

        // VRAM F (MST == 4)
        if self.vramcnt_f.enabled {
            let offset = if self.vramcnt_f.offset & 0x1 != 0 {
                1024 * 16
            } else {
                0
            };
            if addr_in_range(address, offset, VRAM_F_SIZE as u32) && self.vramcnt_f.mst == 4 {
                if let (Some(&lo), Some(&hi)) = (
                    self.vram_f.get((address & VRAM_F_MASK) as usize),
                    self.vram_f.get(((address & VRAM_F_MASK) + 1) as usize),
                ) {
                    value |= u16::from_le_bytes([lo, hi]);
                }
            }
        }

        // VRAM G (MST == 4)
        if self.vramcnt_g.enabled {
            let offset = if self.vramcnt_g.offset & 0x1 != 0 {
                1024 * 16
            } else {
                0
            };
            if addr_in_range(address, offset, VRAM_G_SIZE as u32) && self.vramcnt_g.mst == 4 {
                if let (Some(&lo), Some(&hi)) = (
                    self.vram_g.get((address & VRAM_G_MASK) as usize),
                    self.vram_g.get(((address & VRAM_G_MASK) + 1) as usize),
                ) {
                    value |= u16::from_le_bytes([lo, hi]);
                }
            }
        }

        value
    }

    pub fn read_extpal_bgb_u16(&self, address: u32) -> u16 {
        if !self.vramcnt_h.enabled {
            return 0;
        }

        if !addr_in_range(address, 0, VRAM_H_SIZE as u32) || self.vramcnt_h.mst != 2 {
            #[cfg(feature = "tracing")]
            tracing::error!("Address out of range or MST not 2 in read_extpal_bgb");
            panic!("Address out of range or MST not 2 in read_extpal_bgb");
        }

        let index = (address & VRAM_H_MASK) as usize;
        let lo = self
            .vram_h
            .get(index)
            .copied()
            .expect("VRAM_H index out of bounds");
        let hi = self
            .vram_h
            .get(index + 1)
            .copied()
            .expect("VRAM_H index+1 out of bounds");

        u16::from_le_bytes([lo, hi])
    }

    /// Read extended OBJ palette A (VRAM F/G).
    pub fn read_extpal_obja(&self, address: u32) -> u16 {
        let mut reg: u16 = 0;

        let (f_in_ramge, g_in_range) = (
            addr_in_range(address, 0, VRAM_F_SIZE as u32 / 2) && self.vramcnt_f.mst == 5,
            addr_in_range(address, 0, VRAM_G_SIZE as u32 / 2) && self.vramcnt_g.mst == 5,
        );

        // VRAM F mapping
        if self.vramcnt_f.enabled && f_in_ramge {
            let addr = (address & VRAM_F_MASK) as usize;
            reg |= u16::from_le_bytes([self.vram_f[addr], self.vram_f[addr + 1]]);
        }

        // VRAM G mapping
        if self.vramcnt_g.enabled && g_in_range {
            let addr = (address & VRAM_G_MASK) as usize;
            reg |= u16::from_le_bytes([self.vram_g[addr], self.vram_g[addr + 1]]);
        }
        reg
    }

    /// Read extended OBJ palette B (VRAM I).
    pub fn read_extpal_objb(&self, address: u32) -> u16 {
        let mut reg: u16 = 0;
        let i_in_range =
            addr_in_range(address, 0, VRAM_I_SIZE as u32 / 2) && self.vramcnt_i.mst == 3;

        if self.vramcnt_i.enabled && i_in_range {
            let addr = (address & VRAM_I_MASK) as usize;
            reg |= u16::from_le_bytes([self.vram_i[addr], self.vram_i[addr + 1]]);
        }
        reg
    }
}
