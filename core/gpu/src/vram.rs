// SPDX-License-Identifier: GPL-3.0-or-later
//! VRAM banks A-I and their mapping into the DS address spaces.
//!
//! Every mappable region is split into 16 KiB pages, and each page stores a
//! bit mask of the banks mapped there (bit 0 = A ... bit 8 = I), like melonDS
//! `GPU::VRAMMap_*`. Reads OR together all banks mapped at a page and writes
//! go to every one of them, which is what the hardware does when banks
//! overlap. Banks are always mapped at offsets aligned to their own size, so
//! the position inside a bank is simply `region_offset & (bank_size - 1)`.
//!
//! See GBATEK "DS Memory Control - VRAM".

/// Bank sizes in bytes, A..=I.
pub const BANK_SIZES: [usize; 9] = [
    128 * 1024, // A
    128 * 1024, // B
    128 * 1024, // C
    128 * 1024, // D
    64 * 1024,  // E
    16 * 1024,  // F
    16 * 1024,  // G
    32 * 1024,  // H
    16 * 1024,  // I
];

const PAGE_SHIFT: u32 = 14;
const PAGE_SIZE: u32 = 1 << PAGE_SHIFT;

/// LCDC (plain CPU access) base offset of each bank inside 0x0680_0000.
const LCDC_OFFSETS: [u32; 9] = [
    // DS Memory Control - VRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram
    0x00000, 0x20000, 0x40000, 0x60000, 0x80000, 0x90000, 0x94000, 0x98000,
    0xA0000, // LCDC offsets of banks A-I from 06800000h
];

/// A mappable address space, e.g. engine A BG VRAM.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// Engine A BG (512 KiB at 0x0600_0000)
    BgA,
    /// Engine A OBJ (256 KiB at 0x0640_0000)
    ObjA,
    /// Engine B BG (128 KiB at 0x0620_0000)
    BgB,
    /// Engine B OBJ (128 KiB at 0x0660_0000)
    ObjB,
    /// LCDC (656 KiB at 0x0680_0000)
    Lcdc,
    /// ARM7 WRAM-mapped VRAM (256 KiB at 0x0600_0000 on the ARM7)
    Arm7,
    /// 3D texture image slots 0-3 (512 KiB)
    TexImage,
    /// 3D texture palette slots 0-5 (96 KiB, 128 KiB address space)
    TexPal,
    /// Engine A BG extended palette slots 0-3 (32 KiB)
    BgExtPalA,
    /// Engine B BG extended palette slots 0-3 (32 KiB)
    BgExtPalB,
    /// Engine A OBJ extended palette (8 KiB)
    ObjExtPalA,
    /// Engine B OBJ extended palette (8 KiB)
    ObjExtPalB,
}

impl Region {
    const COUNT: usize = 12;

    /// Size of the region's address space in bytes (accesses wrap around).
    const fn size(self) -> u32 {
        match self {
            Self::BgA | Self::TexImage => 512 * 1024,
            Self::ObjA | Self::Arm7 => 256 * 1024,
            Self::BgB | Self::ObjB | Self::TexPal => 128 * 1024,
            Self::Lcdc => 1024 * 1024,
            Self::BgExtPalA | Self::BgExtPalB => 32 * 1024,
            Self::ObjExtPalA | Self::ObjExtPalB => 16 * 1024,
        }
    }

    /// Number of 16 KB pages covered by the region.
    const fn pages(self) -> usize {
        (self.size() >> PAGE_SHIFT) as usize
    }
}

/// VRAM banks plus their page mapping tables.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct Vram {
    banks: [Vec<u8>; 9],
    /// Raw VRAMCNT_A..I values.
    cnt: [u8; 9],
    /// Per region, per 16 KiB page: mask of mapped banks.
    maps: [Vec<u16>; Region::COUNT],
}

impl Default for Vram {
    fn default() -> Self {
        Self::new()
    }
}

impl Vram {
    /// All banks zeroed and unmapped.
    pub fn new() -> Self {
        Self {
            banks: BANK_SIZES.map(|size| vec![0; size]),
            cnt: [0; 9],
            maps: [
                Region::BgA,
                Region::ObjA,
                Region::BgB,
                Region::ObjB,
                Region::Lcdc,
                Region::Arm7,
                Region::TexImage,
                Region::TexPal,
                Region::BgExtPalA,
                Region::BgExtPalB,
                Region::ObjExtPalA,
                Region::ObjExtPalB,
            ]
            .map(|r| vec![0; r.pages()]),
        }
    }

    /// Clears all bank contents and mappings (power on).
    pub fn reset(&mut self) {
        for bank in &mut self.banks {
            bank.fill(0);
        }
        self.cnt = [0; 9];
        self.remap();
    }

    /// VRAMCNT value of bank `bank` (0 = A).
    pub const fn cnt(&self, bank: usize) -> u8 {
        self.cnt[bank]
    }

    /// Raw bank memory (0 = A).
    pub fn bank(&self, bank: usize) -> &[u8] {
        &self.banks[bank]
    }

    /// Raw mutable bank memory (0 = A).
    pub fn bank_mut(&mut self, bank: usize) -> &mut [u8] {
        &mut self.banks[bank]
    }

    /// Writes VRAMCNT for bank `bank` (0 = A) and rebuilds the mapping.
    pub fn set_cnt(&mut self, bank: usize, value: u8) {
        // Bits 3-4 (offset) do not exist for E/H/I; MST is 2 bits for A/B/H/I.
        let mask = match bank {
            // DS Memory Control - VRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram
            0 | 1 => 0x9B, // banks A/B: enable (bit 7), OFS (bits 3-4), MST (bits 0-1)
            4 => 0x87,     // bank E: enable (bit 7), MST (bits 0-2), no OFS
            7 | 8 => 0x83, // banks H/I: enable (bit 7), MST (bits 0-1), no OFS
            _ => 0x9F,     // banks C/D/F/G: enable (bit 7), OFS (bits 3-4), MST (bits 0-2)
        };
        self.cnt[bank] = value & mask;
        self.remap();
    }

    /// VRAMSTAT (ARM7 0x0400_0240): bit 0/1 = bank C/D mapped to the ARM7.
    pub const fn vramstat(&self) -> u8 {
        let c = self.cnt[2];
        let d = self.cnt[3];
        // DS Memory Control - VRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram
        ((c & 0x87 == 0x82) as u8) | (((d & 0x87 == 0x82) as u8) << 1) // VRAMSTAT: bank C/D enabled with MST=2 (mapped to ARM7)
    }

    /// Maps `bank` into `region` pages `start..start + size`.
    fn map(&mut self, region: Region, start: u32, size: u32, bank: usize) {
        let first = (start >> PAGE_SHIFT) as usize;
        let count = (size.div_ceil(PAGE_SIZE)) as usize;
        let pages = &mut self.maps[region as usize];
        for page in first..first + count {
            if let Some(p) = pages.get_mut(page) {
                *p |= 1 << bank;
            }
        }
    }

    /// Rebuilds every page table from the VRAMCNT registers.
    fn remap(&mut self) {
        for pages in &mut self.maps {
            pages.fill(0);
        }

        for bank in 0..9 {
            let cnt = self.cnt[bank];
            // DS Memory Control - VRAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram
            if cnt & 0x80 == 0 {
                // bit 7: bank enable
                continue;
            }
            let mst = u32::from(cnt & 0x7); // MST - VRAM bank mode (bits 0-2)
            let ofs = u32::from((cnt >> 3) & 0x3); // OFS - VRAM bank offset (bits 3-4)
            let size = BANK_SIZES[bank] as u32;

            match (bank, mst) {
                (_, 0) => self.map(Region::Lcdc, LCDC_OFFSETS[bank], size, bank),

                // A, B
                (0 | 1, 1) => self.map(Region::BgA, ofs * 0x20000, size, bank), // MST 1: engine A BG VRAM at 06000000h + 20000h*OFS
                (0 | 1, 2) => self.map(Region::ObjA, (ofs & 1) * 0x20000, size, bank), // MST 2: engine A OBJ VRAM at 06400000h + 20000h*OFS.0
                (0 | 1, 3) => self.map(Region::TexImage, ofs * 0x20000, size, bank), // MST 3: 3D texture slot OFS

                // C
                (2, 1) => self.map(Region::BgA, ofs * 0x20000, size, bank), // MST 1: engine A BG VRAM at 06000000h + 20000h*OFS
                (2, 2) => self.map(Region::Arm7, (ofs & 1) * 0x20000, size, bank), // MST 2: ARM7 WRAM at 06000000h + 20000h*OFS.0
                (2, 3) => self.map(Region::TexImage, ofs * 0x20000, size, bank), // MST 3: 3D texture slot OFS
                (2, 4) => self.map(Region::BgB, 0, size, bank),

                // D
                (3, 1) => self.map(Region::BgA, ofs * 0x20000, size, bank), // MST 1: engine A BG VRAM at 06000000h + 20000h*OFS
                (3, 2) => self.map(Region::Arm7, (ofs & 1) * 0x20000, size, bank), // MST 2: ARM7 WRAM at 06000000h + 20000h*OFS.0
                (3, 3) => self.map(Region::TexImage, ofs * 0x20000, size, bank), // MST 3: 3D texture slot OFS
                (3, 4) => self.map(Region::ObjB, 0, size, bank),

                // E
                (4, 1) => self.map(Region::BgA, 0, size, bank),
                (4, 2) => self.map(Region::ObjA, 0, size, bank),
                (4, 3) => self.map(Region::TexPal, 0, size, bank),
                (4, 4) => self.map(Region::BgExtPalA, 0, 0x8000, bank), // bank E MST 4: engine A BG extended palette slots 0-3 (32K)

                // F, G
                (5 | 6, 1) => {
                    let start = (ofs & 1) * 0x4000 + (ofs >> 1) * 0x10000; // 06000000h + 4000h*OFS.0 + 10000h*OFS.1
                    self.map(Region::BgA, start, size, bank);
                }
                (5 | 6, 2) => {
                    let start = (ofs & 1) * 0x4000 + (ofs >> 1) * 0x10000; // 06400000h + 4000h*OFS.0 + 10000h*OFS.1
                    self.map(Region::ObjA, start, size, bank);
                }
                (5 | 6, 3) => {
                    let slot = (ofs & 1) + (ofs >> 1) * 4;
                    self.map(Region::TexPal, slot * 0x4000, size, bank); // texture palette slot (OFS.0 + OFS.1 * 4) * 16K
                }
                (5 | 6, 4) => self.map(Region::BgExtPalA, (ofs & 1) * 0x4000, size, bank), // MST 4: engine A BG extended palette slot 0/1 or 2/3
                (5 | 6, 5) => self.map(Region::ObjExtPalA, 0, size, bank),

                // H
                (7, 1) => self.map(Region::BgB, 0, size, bank),
                (7, 2) => self.map(Region::BgExtPalB, 0, size, bank),

                // I
                (8, 1) => self.map(Region::BgB, 0x8000, size, bank), // bank I MST 1: engine B BG VRAM at 06208000h
                (8, 2) => self.map(Region::ObjB, 0, size, bank),
                (8, 3) => self.map(Region::ObjExtPalB, 0, size, bank),

                _ => {}
            }
        }
    }

    /// Reads `N` bytes at `offset` inside `region` (offset wraps at the
    /// region size). Unmapped pages read as 0.
    #[inline(always)]
    pub fn read<const N: usize>(&self, region: Region, offset: u32) -> [u8; N] {
        let offset = offset & (region.size() - 1);
        let mut mask = self.maps[region as usize][(offset >> PAGE_SHIFT) as usize];
        // Usual case, one bank mapped: copy straight out of it.
        if mask.is_power_of_two() {
            let mem = &self.banks[mask.trailing_zeros() as usize];
            let start = offset as usize & (mem.len() - 1);
            return match mem.get(start..start + N) {
                Some(src) => core::array::from_fn(|i| src[i]),
                None => [0; N],
            };
        }
        let mut out = [0u8; N];
        while mask != 0 {
            let bank = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let mem = &self.banks[bank];
            let start = offset as usize & (mem.len() - 1);
            if let Some(src) = mem.get(start..start + N) {
                for (o, s) in out.iter_mut().zip(src) {
                    *o |= *s;
                }
            }
        }
        out
    }

    /// Writes `bytes` at `offset` inside `region` to every mapped bank.
    #[inline]
    pub fn write(&mut self, region: Region, offset: u32, bytes: &[u8]) {
        let offset = offset & (region.size() - 1);
        let mut mask = self.maps[region as usize][(offset >> PAGE_SHIFT) as usize];
        while mask != 0 {
            let bank = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let mem = &mut self.banks[bank];
            let start = offset as usize & (mem.len() - 1);
            if let Some(dst) = mem.get_mut(start..start + bytes.len()) {
                dst.copy_from_slice(bytes);
            }
        }
    }

    /// Copies a whole region (as seen by its user, e.g. the 3D engine's
    /// texture slots) into `out`, which must be the region's size.
    /// Unmapped pages read as 0.
    pub fn copy_region(&self, region: Region, out: &mut [u8]) {
        out.fill(0);
        for (page, chunk) in out.chunks_mut(PAGE_SIZE as usize).enumerate() {
            let offset = page << PAGE_SHIFT;
            let mut mask = self.maps[region as usize][page];
            while mask != 0 {
                let bank = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let mem = &self.banks[bank];
                let start = offset & (mem.len() - 1);
                if let Some(src) = mem.get(start..start + chunk.len()) {
                    for (o, s) in chunk.iter_mut().zip(src) {
                        *o |= *s;
                    }
                }
            }
        }
    }

    /// Byte read from `region` through the bank mapping (0 if unmapped).
    #[inline]
    pub fn read_u8(&self, region: Region, offset: u32) -> u8 {
        self.read::<1>(region, offset)[0]
    }

    /// Halfword read from `region` through the bank mapping.
    #[inline]
    pub fn read_u16(&self, region: Region, offset: u32) -> u16 {
        u16::from_le_bytes(self.read(region, offset))
    }

    /// Word read from `region` through the bank mapping.
    #[inline]
    pub fn read_u32(&self, region: Region, offset: u32) -> u32 {
        u32::from_le_bytes(self.read(region, offset))
    }

    /// Doubleword read from `region` through the bank mapping.
    #[inline]
    pub fn read_u64(&self, region: Region, offset: u32) -> u64 {
        u64::from_le_bytes(self.read(region, offset))
    }

    /// Resolves an ARM9 address in 0x0600_0000..0x0700_0000 to its region.
    pub const fn arm9_region(address: u32) -> (Region, u32) {
        // DS Memory Maps: https://problemkaputt.de/gbatek.htm#dsmemorymaps
        let offset = address & 0x1F_FFFF; // offset inside a 2MB VRAM window
        let region = match address & 0x00E0_0000 {
            // address bits 21-23 select the window
            0x0000_0000 => Region::BgA,  // 06000000h engine A BG VRAM
            0x0020_0000 => Region::BgB,  // 06200000h engine B BG VRAM
            0x0040_0000 => Region::ObjA, // 06400000h engine A OBJ VRAM
            0x0060_0000 => Region::ObjB, // 06600000h engine B OBJ VRAM
            _ => Region::Lcdc,
        };
        let offset = match region {
            Region::Lcdc => address & 0xF_FFFF, // 06800000h LCDC VRAM (1MB window)
            _ => offset,
        };
        (region, offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_c_as_engine_b_bg() {
        let mut vram = Vram::new();
        vram.set_cnt(2, 0x84); // VRAMCNT_C = 84h: enable, MST 4 (engine B BG)
        let (region, off) = Vram::arm9_region(0x0620_0010); // engine B BG VRAM
        vram.write(region, off, &0xBEEF_u16.to_le_bytes()); // test pattern
        assert_eq!(vram.read_u16(Region::BgB, 0x10), 0xBEEF);
        // mirrored every 128 KiB inside the engine B BG window
        assert_eq!(vram.read_u16(Region::BgB, 0x2_0010), 0xBEEF);
        assert_eq!(vram.bank(2)[0x10], 0xEF);
        // not visible through engine A or LCDC
        assert_eq!(vram.read_u16(Region::BgA, 0x10), 0);
        assert_eq!(vram.read_u16(Region::Lcdc, 0x4_0010), 0); // LCDC offset of bank C
    }

    #[test]
    fn bank_f_offsets() {
        for (ofs, start) in [(0u8, 0u32), (1, 0x4000), (2, 0x1_0000), (3, 0x1_4000)] {
            // (OFS, mapped start) for 06000000h + 4000h*OFS.0 + 10000h*OFS.1
            let mut vram = Vram::new();
            vram.set_cnt(5, 0x81 | (ofs << 3)); // enable + MST 1, OFS in bits 3-4
            vram.write(Region::BgA, start + 2, &[0x5A]); // test pattern
            assert_eq!(vram.bank(5)[2], 0x5A, "ofs {ofs}"); // test pattern
            assert_eq!(vram.read_u8(Region::BgA, start + 2), 0x5A); // test pattern
            let other = if start == 0 { 0x4000 } else { 0 }; // the other 16K slot
            assert_eq!(vram.read_u8(Region::BgA, other + 2), 0, "ofs {ofs}");
        }
    }

    #[test]
    fn banks_c_d_on_arm7_and_vramstat() {
        let mut vram = Vram::new();
        vram.set_cnt(2, 0x82); // C -> ARM7 slot 0
        vram.set_cnt(3, 0x8A); // D -> ARM7 slot 1
        vram.write(Region::Arm7, 0x0, &[1]); // bank C slot (OFS 0)
        vram.write(Region::Arm7, 0x2_0000, &[2]); // bank D slot (OFS 1)
        assert_eq!(vram.bank(2)[0], 1);
        assert_eq!(vram.bank(3)[0], 2);
        assert_eq!(vram.vramstat(), 0b11);
    }

    #[test]
    fn lcdc_and_overlap() {
        let mut vram = Vram::new();
        vram.set_cnt(0, 0x80); // A -> LCDC
        vram.write(Region::Lcdc, 0x100, &[7, 8]); // LCDC offset within bank A
        assert_eq!(vram.read_u16(Region::Lcdc, 0x100), 0x0807);
        // A and B both as BG-A slot 0: reads OR, writes hit both
        vram.set_cnt(0, 0x81);
        vram.set_cnt(1, 0x81); // bank B: enable + MST 1, overlapping bank A
        vram.write(Region::BgA, 0x200, &[0x0F]); // test pattern
        assert_eq!(vram.bank(0)[0x200], 0x0F); // test pattern
        assert_eq!(vram.bank(1)[0x200], 0x0F); // test pattern
    }

    #[test]
    fn ext_palettes() {
        let mut vram = Vram::new();
        vram.set_cnt(4, 0x84); // E -> BG ext pal A slots 0-3
        vram.bank_mut(4)[0x6002] = 0x33; // bank E offset 6000h = BG extended palette slot 3
        assert_eq!(vram.read_u8(Region::BgExtPalA, 0x6002), 0x33); // test pattern
        vram.set_cnt(7, 0x82); // H -> BG ext pal B
        vram.bank_mut(7)[0x2004] = 0x44; // bank H offset 2000h = BG extended palette slot 1
        assert_eq!(vram.read_u8(Region::BgExtPalB, 0x2004), 0x44); // test pattern
    }
}
