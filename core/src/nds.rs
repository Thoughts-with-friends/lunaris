use crate::likely;
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

use crate::arm::ARM;
use crate::hw::HW;

pub use crate::hw::{Engine, GraphicsType, Key};

pub struct NDS {
    arm7: ARM<false>,
    arm9: ARM<true>,
    hw: HW,
}

impl NDS {
    pub const CLOCK_RATE: usize = 33513982;

    pub fn new(
        bios7: Vec<u8>,
        bios9: Vec<u8>,
        firmware_file: File,
        rom: Vec<u8>,
        save_file: File,
    ) -> Self {
        let direct_boot = true;
        let mut hw = HW::new(bios7, bios9, firmware_file, rom, save_file, direct_boot);
        NDS {
            arm7: ARM::new(&mut hw, direct_boot),
            arm9: ARM::new(&mut hw, direct_boot),
            hw,
        }
    }

    pub fn emulate_frame(&mut self) {
        while !self.hw.rendered_frame() {
            if likely(!self.hw.gpu.bus_stalled()) {
                let cycle = self.hw.cycle();
                // The max cycle desync was ~30 when the CPUs were running tightly
                let target = std::cmp::min(cycle + 30, self.hw.cycle_at_next_event());

                self.arm9.emulate(&mut self.hw, target * 2);
                self.arm7.emulate(&mut self.hw, target);
                self.hw.clock_until(target);
            } else {
                self.hw.clock_until_event();
                self.arm9.set_cycle(self.hw.cycle() * 2);
                self.arm7.set_cycle(self.hw.cycle());
            }
        }
    }

    #[inline]
    pub fn get_screens(&self) -> [&Vec<u16>; 2] {
        self.hw.gpu.get_screens()
    }

    #[inline]
    pub fn press_key(&mut self, key: Key) {
        self.hw.press_key(key);
    }

    #[inline]
    pub fn release_key(&mut self, key: Key) {
        self.hw.release_key(key);
    }

    #[inline]
    pub fn press_screen(&mut self, x: usize, y: usize) {
        self.hw.press_screen(x, y);
    }

    #[inline]
    pub fn release_screen(&mut self) {
        self.hw.release_screen();
    }

    #[inline]
    pub fn render_palettes(
        &self,
        extended: bool,
        slot: usize,
        palette: usize,
        engine: Engine,
        graphics_type: GraphicsType,
    ) -> (Vec<u16>, usize, usize) {
        self.hw
            .render_palettes(extended, slot, palette, engine, graphics_type)
    }

    #[inline]
    pub fn render_map(&self, engine: Engine, bg_i: usize) -> (Vec<u16>, usize, usize) {
        self.hw.render_map(engine, bg_i)
    }

    #[inline]
    pub fn render_tiles(
        &self,
        engine: Engine,
        graphics_type: GraphicsType,
        extended: bool,
        bitmap: bool,
        bpp8: bool,
        slot: usize,
        palette: usize,
        offset: usize,
    ) -> (Vec<u16>, usize, usize) {
        self.hw.render_tiles(
            engine,
            graphics_type,
            extended,
            bitmap,
            bpp8,
            slot,
            palette,
            offset,
        )
    }

    #[inline]
    pub fn render_bank(&self, bank: usize, ignore_alpha: bool) -> (Vec<u16>, usize, usize) {
        self.hw.render_bank(ignore_alpha, bank)
    }

    pub fn load_rom(
        bios7_path: Option<&Path>,
        bios9_path: Option<&Path>,
        firmware_path: Option<&Path>,
        rom_path: &Path,
    ) -> Self {
        let save_file_path = rom_path.with_extension("sav");

        let save_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&save_file_path)
            .unwrap();

        let bios7 = bios7_path
            .and_then(|path| fs::read(path).ok())
            .unwrap_or_else(|| free_bios::arm7::BIOS_ARM7_BIN.to_vec());

        let bios9 = bios9_path
            .and_then(|path| fs::read(path).ok())
            .unwrap_or_else(|| free_bios::arm9::BIOS_ARM9_BIN.to_vec());

        let firmware_file = if let Some(path) = firmware_path {
            match OpenOptions::new().read(true).write(true).open(path) {
                Ok(file) => file,
                Err(_) => {
                    fs::write(path, free_bios::firmware::FIRMWARE_DS).unwrap();

                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(path)
                        .unwrap()
                }
            }
        } else {
            let firmware_path = std::env::temp_dir().join("freebios_firmware.bin");

            if !firmware_path.exists() {
                fs::write(&firmware_path, free_bios::firmware::FIRMWARE_DS).unwrap();
            }

            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&firmware_path)
                .unwrap()
        };

        NDS::new(
            bios7,
            bios9,
            firmware_file,
            fs::read(rom_path).unwrap(),
            save_file,
        )
    }
}

pub const WIDTH: usize = crate::hw::GPU::WIDTH;
pub const HEIGHT: usize = crate::hw::GPU::HEIGHT;
