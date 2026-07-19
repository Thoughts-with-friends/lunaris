//! VRAM banks A-I and their dynamic mapping (VRAMCNT).
//!
//! The nine banks (A-D: 128 KiB, E: 64 KiB, F/G/I: 16 KiB, H: 32 KiB) can be
//! mapped as LCDC "plain" VRAM, engine A/B BG/OBJ memory, extended
//! palettes, 3D textures / texture palettes, or ARM7 work RAM, selected by
//! each bank's MST+OFS fields in VRAMCNT.
//!
//! GBATEK "DS Memory Control - VRAM" (VRAMCNT_A..I, VRAMSTAT, all mapping
//! tables): <https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram>

use num_traits as num;

use super::{
    super::{HW, MemoryValue},
    EngineType,
};

/// `Vec::load_in_place` does not consume the stored length prefix, so every
/// `Vec`/array-of-`Vec` field below routes through `Loadable` instead.
/// See `docs/design/savestate-and-video-design.md`.
#[derive(emu_utils::Savestate)]
pub struct VRAM {
    cnts: [VRAMCNT; 9],
    #[load(with = "save.load()?", with_in_place = "*banks = save.load()?")]
    pub(super) banks: [Vec<u8>; 9],
    // Functions
    lcdc_enabled: [bool; 9],
    #[load(with = "save.load()?", with_in_place = "*lcdc = save.load()?")]
    lcdc: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*engine_a_bg = save.load()?")]
    engine_a_bg: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*engine_a_obj = save.load()?")]
    engine_a_obj: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*engine_a_bg_ext_pal = save.load()?")]
    engine_a_bg_ext_pal: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*engine_a_obj_ext_pal = save.load()?")]
    engine_a_obj_ext_pal: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*textures = save.load()?")]
    textures: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*textures_pal = save.load()?")]
    textures_pal: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*engine_b_bg = save.load()?")]
    engine_b_bg: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*engine_b_obj = save.load()?")]
    engine_b_obj: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*engine_b_bg_ext_pal = save.load()?")]
    engine_b_bg_ext_pal: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*engine_b_obj_ext_pal = save.load()?")]
    engine_b_obj_ext_pal: Vec<Vec<Bank>>,
    #[load(with = "save.load()?", with_in_place = "*arm7_wram = save.load()?")]
    arm7_wram: Vec<Vec<Bank>>,
}

impl VRAM {
    const BANKS_LEN: [usize; 9] = [
        128 * 0x400,
        128 * 0x400,
        128 * 0x400,
        128 * 0x400,
        64 * 0x400,
        16 * 0x400,
        16 * 0x400,
        32 * 0x400,
        16 * 0x400,
    ];
    const MAPPING_LEN: usize = 16 * 0x400;

    const LCDC_OFFSETS: [usize; 9] =
        [0x0_0000, 0x2_0000, 0x4_0000, 0x6_0000, 0x8_0000, 0x9_0000, 0x9_4000, 0x9_8000, 0xA_0000];
    const ENGINE_A_BG_OFFSET: usize = 0x00_0000;
    const ENGINE_A_OBJ_OFFSET: usize = 0x40_0000;
    const ENGINE_B_BG_OFFSET: usize = 0x20_0000;
    const ENGINE_B_OBJ_OFFSET: usize = 0x60_0000;
    const LCDC_OFFSET: usize = 0x80_0000;

    const ENGINE_A_BG_MASK: usize = (4 * 128 / 16) - 1;
    const ENGINE_A_OBJ_MASK: usize = (2 * 128 / 16) - 1;
    const ENGINE_B_BG_MASK: usize = (128 / 16) - 1;
    const ENGINE_B_OBJ_MASK: usize = (128 / 16) - 1;

    // Can't cast in match :(
    const BANK_A: usize = Bank::A as usize;
    const BANK_B: usize = Bank::B as usize;
    const BANK_C: usize = Bank::C as usize;
    const BANK_D: usize = Bank::D as usize;
    const BANK_E: usize = Bank::E as usize;
    const BANK_F: usize = Bank::F as usize;
    const BANK_G: usize = Bank::G as usize;
    const BANK_H: usize = Bank::H as usize;
    const BANK_I: usize = Bank::I as usize;

    pub fn new() -> Self {
        let create_vecs = |len| {
            std::iter::repeat_n(Vec::with_capacity(VRAM::BANKS_LEN.len()), len).collect::<Vec<_>>()
        };
        VRAM {
            cnts: [VRAMCNT::new(0, 0); 9],
            banks: [
                vec![0; VRAM::BANKS_LEN[0]],
                vec![0; VRAM::BANKS_LEN[1]],
                vec![0; VRAM::BANKS_LEN[2]],
                vec![0; VRAM::BANKS_LEN[3]],
                vec![0; VRAM::BANKS_LEN[4]],
                vec![0; VRAM::BANKS_LEN[5]],
                vec![0; VRAM::BANKS_LEN[6]],
                vec![0; VRAM::BANKS_LEN[7]],
                vec![0; VRAM::BANKS_LEN[8]],
            ],
            // Functions
            lcdc_enabled: [false; 9],
            lcdc: create_vecs(41),
            engine_a_bg: create_vecs(32),
            engine_a_obj: create_vecs(16),
            engine_a_bg_ext_pal: create_vecs(2),
            engine_a_obj_ext_pal: create_vecs(1),
            textures: create_vecs(32),
            textures_pal: create_vecs(6),
            engine_b_bg: create_vecs(8),
            engine_b_obj: create_vecs(8),
            engine_b_bg_ext_pal: create_vecs(2),
            engine_b_obj_ext_pal: create_vecs(1),
            arm7_wram: create_vecs(2),
        }
    }

    /// Reads VRAMCNT_A..I (4000240h..4000249h, NDS9, write-only on hardware
    /// but readable here for debugging).
    ///
    /// GBATEK: <https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram>
    pub fn read_vram_cnt(&self, index: usize) -> u8 {
        self.cnts[index].read()
    }

    /// Writes VRAMCNT for bank `index`: unmaps the bank from its previous
    /// function and remaps it according to the new MST/OFS combination.
    ///
    /// The `(bank, mst)` match arms below mirror the per-bank mapping
    /// tables in GBATEK "DS Memory Control - VRAM":
    /// <https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram>
    pub fn write_vram_cnt(&mut self, index: usize, value: u8) {
        let bank = Bank::from_index(index);
        let new_cnt = VRAMCNT::new(index, value);

        if self.cnts[index].enabled {
            match (index, self.cnts[index].mst) {
                (index, 0) => {
                    assert!(self.lcdc_enabled[index]);
                    self.lcdc_enabled[index] = false;
                    VRAM::remove_mapping(&mut self.lcdc, bank, VRAM::LCDC_OFFSETS[index], None)
                }
                (VRAM::BANK_A..=VRAM::BANK_G, 1) => VRAM::remove_mapping(
                    &mut self.engine_a_bg,
                    bank,
                    bank.get_engine_a_offset(self.cnts[index].offset),
                    None,
                ),
                // TODO: Replace with match or syntax
                (VRAM::BANK_A..=VRAM::BANK_B, 2) | (VRAM::BANK_E..=VRAM::BANK_G, 2) => {
                    VRAM::remove_mapping(
                        &mut self.engine_a_obj,
                        bank,
                        bank.get_engine_a_offset(self.cnts[index].offset),
                        None,
                    )
                }
                (VRAM::BANK_E, 4) => {
                    VRAM::remove_mapping(&mut self.engine_a_bg_ext_pal, bank, 0, Some(32 * 0x400))
                }
                (VRAM::BANK_F..=VRAM::BANK_G, 4) => VRAM::remove_mapping(
                    &mut self.engine_a_bg_ext_pal,
                    bank,
                    bank.get_ext_bg_pal_offset(self.cnts[index].offset),
                    None,
                ),
                (VRAM::BANK_F..=VRAM::BANK_G, 5) => {
                    VRAM::remove_mapping(&mut self.engine_a_obj_ext_pal, bank, 0, None)
                }
                (VRAM::BANK_C, 4) | (VRAM::BANK_H, 1) => {
                    VRAM::remove_mapping(&mut self.engine_b_bg, bank, 0, None)
                }
                (VRAM::BANK_I, 1) => {
                    VRAM::remove_mapping(&mut self.engine_b_bg, bank, 0x8000, None)
                }
                (VRAM::BANK_D, 4) | (VRAM::BANK_I, 2) => {
                    VRAM::remove_mapping(&mut self.engine_b_obj, bank, 0, None)
                }
                (VRAM::BANK_H, 2) => {
                    VRAM::remove_mapping(&mut self.engine_b_bg_ext_pal, bank, 0, None)
                }
                (VRAM::BANK_I, 3) => {
                    VRAM::remove_mapping(&mut self.engine_b_obj_ext_pal, bank, 0, Some(8 * 0x400))
                }
                (VRAM::BANK_C..=VRAM::BANK_D, 2) => {
                    self.remove_arm7_wram_mapping(bank, self.cnts[index].offset)
                }
                (VRAM::BANK_A..=VRAM::BANK_D, 3) => VRAM::remove_mapping(
                    &mut self.textures,
                    bank,
                    bank.get_textures_offset(self.cnts[index].offset),
                    None,
                ),
                (VRAM::BANK_E, 3) => VRAM::remove_mapping(&mut self.textures_pal, bank, 0, None),
                (VRAM::BANK_F..=VRAM::BANK_G, 3) => VRAM::remove_mapping(
                    &mut self.textures_pal,
                    bank,
                    bank.get_textures_pal_offset(self.cnts[index].offset),
                    None,
                ),
                _ => unreachable!(),
            }
        }

        self.cnts[index] = new_cnt;
        if !new_cnt.enabled {
            return;
        }

        match (index, new_cnt.mst) {
            (index, 0) => {
                assert!(!self.lcdc_enabled[index]);
                self.lcdc_enabled[index] = true;
                VRAM::add_mapping(&mut self.lcdc, bank, VRAM::LCDC_OFFSETS[index], None)
            }
            (VRAM::BANK_A..=VRAM::BANK_G, 1) => VRAM::add_mapping(
                &mut self.engine_a_bg,
                bank,
                bank.get_engine_a_offset(new_cnt.offset),
                None,
            ),
            // TODO: Replace with match or syntax
            (VRAM::BANK_A..=VRAM::BANK_B, 2) | (VRAM::BANK_E..=VRAM::BANK_G, 2) => {
                VRAM::add_mapping(
                    &mut self.engine_a_obj,
                    bank,
                    bank.get_engine_a_offset(new_cnt.offset),
                    None,
                )
            }
            (VRAM::BANK_E, 4) => {
                VRAM::add_mapping(&mut self.engine_a_bg_ext_pal, bank, 0, Some(32 * 0x400))
            }
            (VRAM::BANK_F..=VRAM::BANK_G, 4) => VRAM::add_mapping(
                &mut self.engine_a_bg_ext_pal,
                bank,
                bank.get_ext_bg_pal_offset(self.cnts[index].offset),
                None,
            ),
            (VRAM::BANK_F..=VRAM::BANK_G, 5) => {
                VRAM::add_mapping(&mut self.engine_a_obj_ext_pal, bank, 0, None)
            }
            (VRAM::BANK_C, 4) | (VRAM::BANK_H, 1) => {
                VRAM::add_mapping(&mut self.engine_b_bg, bank, 0, None)
            }
            (VRAM::BANK_I, 1) => VRAM::add_mapping(&mut self.engine_b_bg, bank, 0x8000, None),
            (VRAM::BANK_D, 4) | (VRAM::BANK_I, 2) => {
                VRAM::add_mapping(&mut self.engine_b_obj, bank, 0, None)
            }
            (VRAM::BANK_H, 2) => VRAM::add_mapping(&mut self.engine_b_bg_ext_pal, bank, 0, None),
            (VRAM::BANK_I, 3) => {
                VRAM::add_mapping(&mut self.engine_b_obj_ext_pal, bank, 0, Some(8 * 0x400))
            }
            (VRAM::BANK_C..=VRAM::BANK_D, 2) => {
                self.add_arm7_wram_mapping(bank, self.cnts[index].offset)
            }
            (VRAM::BANK_A..=VRAM::BANK_D, 3) => VRAM::add_mapping(
                &mut self.textures,
                bank,
                bank.get_textures_offset(self.cnts[index].offset),
                None,
            ),
            (VRAM::BANK_E, 3) => VRAM::add_mapping(&mut self.textures_pal, bank, 0, None),
            (VRAM::BANK_F..=VRAM::BANK_G, 3) => VRAM::add_mapping(
                &mut self.textures_pal,
                bank,
                bank.get_textures_pal_offset(self.cnts[index].offset),
                None,
            ),
            _ => unreachable!(),
        }
    }

    pub fn arm7_read<T: MemoryValue>(&self, addr: u32) -> T {
        let addr = (addr as usize) & (2 * VRAM::BANKS_LEN[VRAM::BANK_C] - 1);
        let index = addr / VRAM::BANKS_LEN[VRAM::BANK_C];
        let addr = addr & (VRAM::BANKS_LEN[VRAM::BANK_C] - 1);
        let mut value = num::zero();
        for bank in self.arm7_wram[index].iter() {
            value |= HW::read_mem(&self.banks[*bank as usize], addr as u32);
        }
        value
    }

    pub fn arm9_read<T: MemoryValue>(&self, addr: u32) -> T {
        // TODO: Optimize - Slower than previous approach
        let index = addr as usize / VRAM::MAPPING_LEN;
        let addr = addr as usize;
        match addr & 0x00E0_0000 {
            VRAM::ENGINE_A_BG_OFFSET => VRAM::read_mapping(
                &self.banks,
                &self.engine_a_bg[index & VRAM::ENGINE_A_BG_MASK],
                addr,
            ),
            VRAM::ENGINE_B_BG_OFFSET => VRAM::read_mapping(
                &self.banks,
                &self.engine_b_bg[index & VRAM::ENGINE_B_BG_MASK],
                addr,
            ),
            VRAM::ENGINE_A_OBJ_OFFSET => VRAM::read_mapping(
                &self.banks,
                &self.engine_a_obj[index & VRAM::ENGINE_A_OBJ_MASK],
                addr,
            ),
            VRAM::ENGINE_B_OBJ_OFFSET => VRAM::read_mapping(
                &self.banks,
                &self.engine_b_obj[index & VRAM::ENGINE_B_OBJ_MASK],
                addr,
            ),
            // GBATEK's VRAM address decoding for ARM9 only distinguishes 5
            // of the 8 values `addr & 0x00E0_0000` can take (the CPU-side
            // VRAM window is a full 16 MiB, `0x0600_0000`-`0x06FF_FFFF`, far
            // larger than the ~656 KiB of physical VRAM banks). Addresses at
            // or above `LCDC_OFFSET` that aren't one of the other four
            // regions all alias back into LCDC VRAM on real hardware, so
            // every remaining value in the 3-bit selector routes here too.
            VRAM::LCDC_OFFSET..=0x00E0_0000 => VRAM::read_mapping(
                &self.banks,
                &self.lcdc[((addr & 0xF_C000) / VRAM::MAPPING_LEN) % self.lcdc.len()],
                addr,
            ),
            _ => unreachable!(),
        }
    }

    pub fn arm9_write<T: MemoryValue>(&mut self, addr: u32, value: T) {
        // TODO: Optimize - Slower than previous approach
        let index = addr as usize / VRAM::MAPPING_LEN;
        let addr = addr as usize;
        match addr & 0x00E0_0000 {
            VRAM::ENGINE_A_BG_OFFSET => VRAM::write_mapping(
                &mut self.banks,
                &self.engine_a_bg[index % self.engine_a_bg.len()],
                addr,
                value,
            ),
            VRAM::ENGINE_B_BG_OFFSET => VRAM::write_mapping(
                &mut self.banks,
                &self.engine_b_bg[index % self.engine_b_bg.len()],
                addr,
                value,
            ),
            VRAM::ENGINE_A_OBJ_OFFSET => VRAM::write_mapping(
                &mut self.banks,
                &self.engine_a_obj[index % self.engine_a_obj.len()],
                addr,
                value,
            ),
            VRAM::ENGINE_B_OBJ_OFFSET => VRAM::write_mapping(
                &mut self.banks,
                &self.engine_b_obj[index % self.engine_b_obj.len()],
                addr,
                value,
            ),
            // See the matching comment in `arm9_read`.
            VRAM::LCDC_OFFSET..=0x00E0_0000 => VRAM::write_mapping(
                &mut self.banks,
                &self.lcdc[((addr & 0xF_C000) / VRAM::MAPPING_LEN) % self.lcdc.len()],
                addr,
                value,
            ),
            _ => unreachable!(),
        }
    }

    pub fn arm7_write<T: MemoryValue>(&mut self, addr: u32, value: T) {
        let addr = (addr as usize) & (2 * VRAM::BANKS_LEN[VRAM::BANK_C] - 1);
        let index = addr / VRAM::BANKS_LEN[VRAM::BANK_C];
        let addr = addr & (VRAM::BANKS_LEN[VRAM::BANK_C] - 1);
        for bank in self.arm7_wram[index].iter() {
            HW::write_mem(&mut self.banks[*bank as usize], addr as u32, value);
        }
    }

    pub fn get_lcdc_bank(&self, bank: u8) -> Option<&Vec<u8>> {
        if self.lcdc_enabled[bank as usize] { Some(&self.banks[bank as usize]) } else { None }
    }

    pub fn get_bg<E: EngineType, T: MemoryValue>(&self, addr: usize) -> T {
        if E::is_a() {
            VRAM::read_mapping(&self.banks, &self.engine_a_bg[addr / VRAM::MAPPING_LEN], addr)
        } else {
            VRAM::read_mapping(&self.banks, &self.engine_b_bg[addr / VRAM::MAPPING_LEN], addr)
        }
    }

    pub fn get_obj<E: EngineType, T: MemoryValue>(&self, addr: usize) -> T {
        if E::is_a() {
            VRAM::read_mapping(&self.banks, &self.engine_a_obj[addr / VRAM::MAPPING_LEN], addr)
        } else {
            VRAM::read_mapping(&self.banks, &self.engine_b_obj[addr / VRAM::MAPPING_LEN], addr)
        }
    }

    pub fn get_bg_ext_pal<E: EngineType>(&self, slot: usize, color_num: usize) -> u16 {
        let addr = self.calc_ext_pal_addr(slot, color_num);
        if E::is_a() {
            VRAM::read_mapping(
                &self.banks,
                &self.engine_a_bg_ext_pal[addr / VRAM::MAPPING_LEN],
                addr,
            )
        } else {
            VRAM::read_mapping(
                &self.banks,
                &self.engine_b_bg_ext_pal[addr / VRAM::MAPPING_LEN],
                addr,
            )
        }
    }

    pub fn get_obj_ext_pal<E: EngineType>(&self, color_num: usize) -> u16 {
        let addr = self.calc_ext_pal_addr(0, color_num);
        if E::is_a() {
            VRAM::read_mapping(
                &self.banks,
                &self.engine_a_obj_ext_pal[addr / VRAM::MAPPING_LEN],
                addr,
            )
        } else {
            VRAM::read_mapping(
                &self.banks,
                &self.engine_b_obj_ext_pal[addr / VRAM::MAPPING_LEN],
                addr,
            )
        }
    }

    pub fn get_textures<T: MemoryValue>(&self, addr: usize) -> T {
        VRAM::read_mapping(&self.banks, &self.textures[addr / VRAM::MAPPING_LEN], addr)
    }

    pub fn get_textures_pal<T: MemoryValue>(&self, addr: usize) -> T {
        VRAM::read_mapping(&self.banks, &self.textures_pal[addr / VRAM::MAPPING_LEN], addr)
    }

    #[expect(clippy::ptr_arg)]
    fn read_mapping<T: MemoryValue>(banks: &[Vec<u8>], mapping: &Vec<Bank>, addr: usize) -> T {
        let mut value = num::zero();
        for bank in mapping.iter() {
            let addr = addr & (VRAM::BANKS_LEN[*bank as usize] - 1);
            value |= HW::read_mem::<T>(&banks[*bank as usize], addr as u32);
        }
        value
    }

    #[expect(clippy::ptr_arg)]
    fn write_mapping<T: MemoryValue>(
        banks: &mut [Vec<u8>],
        mapping: &Vec<Bank>,
        addr: usize,
        value: T,
    ) {
        for bank in mapping.iter() {
            let addr = addr & (VRAM::BANKS_LEN[*bank as usize] - 1);
            HW::write_mem(&mut banks[*bank as usize], addr as u32, value);
        }
    }

    fn add_mapping(arr: &mut [Vec<Bank>], bank: Bank, offset: usize, size: Option<usize>) {
        let size = size.unwrap_or_else(|| VRAM::BANKS_LEN[bank as usize]);
        for addr in (0..size).step_by(VRAM::MAPPING_LEN) {
            assert!(
                arr[(addr + offset) / VRAM::MAPPING_LEN].iter().position(|b| *b == bank).is_none()
            );
            arr[(addr + offset) / VRAM::MAPPING_LEN].push(bank);
        }
    }

    fn remove_mapping(arr: &mut [Vec<Bank>], bank: Bank, offset: usize, size: Option<usize>) {
        let size = size.unwrap_or_else(|| VRAM::BANKS_LEN[bank as usize]);
        for addr in (0..size).step_by(VRAM::MAPPING_LEN) {
            let vec = &mut arr[(addr + offset) / VRAM::MAPPING_LEN];
            if let Some(pos) = vec.iter().position(|b| *b == bank) {
                vec.swap_remove(pos);
            }
        }
    }

    fn add_arm7_wram_mapping(&mut self, bank: Bank, offset: u8) {
        assert!(offset < 2);
        self.arm7_wram[offset as usize].push(bank);
    }

    fn remove_arm7_wram_mapping(&mut self, bank: Bank, offset: u8) {
        assert!(offset < 2);
        let vec = &mut self.arm7_wram[offset as usize];
        if let Some(pos) = vec.iter().position(|b| *b == bank) {
            vec.swap_remove(pos);
        }
    }

    fn calc_ext_pal_addr(&self, slot: usize, color_num: usize) -> usize {
        slot * 8 * 0x400 + color_num * 2
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug)]
struct VRAMCNT {
    mst: u8,
    offset: u8,
    enabled: bool,
    byte: u8,
}

impl VRAMCNT {
    const MST_MASKS: [u8; 9] = [0x3, 0x3, 0x7, 0x7, 0x7, 0x7, 0x7, 0x3, 0x3];
    const OFS_MASKS: [u8; 9] = [0x3, 0x3, 0x3, 0x3, 0x0, 0x3, 0x3, 0x0, 0x0];

    pub fn new(index: usize, byte: u8) -> Self {
        VRAMCNT {
            mst: byte & VRAMCNT::MST_MASKS[index],
            offset: byte >> 3 & VRAMCNT::OFS_MASKS[index],
            enabled: byte >> 7 & 0x1 != 0,
            byte,
        }
    }

    pub fn read(&self) -> u8 {
        self.byte
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bank {
    A = 0,
    B = 1,
    C = 2,
    D = 3,
    E = 4,
    F = 5,
    G = 6,
    H = 7,
    I = 8,
}

impl Bank {
    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Bank::A,
            1 => Bank::B,
            2 => Bank::C,
            3 => Bank::D,
            4 => Bank::E,
            5 => Bank::F,
            6 => Bank::G,
            7 => Bank::H,
            8 => Bank::I,
            _ => unreachable!(),
        }
    }

    pub fn get_engine_a_offset(&self, offset: u8) -> usize {
        let offset = offset as usize;
        match self {
            Bank::A | Bank::B | Bank::C | Bank::D => 0x2_0000 * offset,
            Bank::E => {
                assert_eq!(offset, 0);
                0
            }
            Bank::F | Bank::G => 0x4000 * (offset & 0x1) + 0x1_0000 * (offset >> 1 & 0x1),
            Bank::H | Bank::I => unreachable!(),
        }
    }

    pub fn get_textures_offset(&self, offset: u8) -> usize {
        offset as usize * 0x400 * 128
    }

    pub fn get_textures_pal_offset(&self, offset: u8) -> usize {
        let slot = ((offset >> 1) & 0x1) * 4 + (offset & 0x1);
        slot as usize * 0x400 * 16
    }

    pub fn get_ext_bg_pal_offset(&self, offset: u8) -> usize {
        assert!(offset < 2); // TODO: Figure out behavior
        offset as usize * 0x400 * 16
    }
}

mod debug {
    use super::{Bank, HW, VRAM};

    impl VRAM {
        pub fn render_bank(&self, ignore_alpha: bool, bank: usize) -> (Vec<u16>, usize, usize) {
            let alpha = if ignore_alpha { 0x8000 } else { 0 };
            let bank = Bank::from_index(bank);
            let (width, height) = match bank {
                Bank::A | Bank::B | Bank::C | Bank::D => (8, 8),
                Bank::E => (8, 4),
                Bank::H => (4, 4),
                Bank::F | Bank::G | Bank::I => (4, 2),
            };
            let mut pixels = vec![0; width * 32 * height * 32];
            // TODO: Cast slice instead of read_mem
            for (i, pixel) in pixels.iter_mut().enumerate() {
                *pixel = alpha | HW::read_mem::<u16>(&self.banks[bank as usize], 2 * i as u32);
            }
            (pixels, width * 32, height * 32)
        }
    }
}
