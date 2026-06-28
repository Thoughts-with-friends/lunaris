use crate::likely;
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

use crate::arm::ARM;
use crate::hw::HW;

pub use crate::hw::{Engine, GraphicsType, Key};

pub const WIDTH: usize = crate::hw::GPU::WIDTH;
pub const HEIGHT: usize = crate::hw::GPU::HEIGHT;

/// Top-level Nintendo DS emulator state.
///
/// Owns both CPU cores and all hardware peripherals.
/// Call [`NDS::emulate_frame`] repeatedly to drive emulation.
#[derive(emu_utils::Savestate)]
#[load(in_place_only)]
pub struct NDS {
    /// ARM7TDMI sub-processor (audio, Wi-Fi, I/O assist).
    arm7: ARM<false>,
    /// ARM946E-S main processor (game code, 3D, main memory).
    arm9: ARM<true>,
    hw: HW,
}

impl NDS {
    /// Deserializes a previously saved state into `self`.
    pub fn load_state(&mut self, state: &[u8]) -> Result<(), emu_utils::ReadError> {
        use emu_utils::ReadSavestate as _;
        let mut reader = emu_utils::PersistentReadSavestate::new(state)
            .map_err(|_| emu_utils::ReadError::InvalidEnum)?;
        reader.load_into(self)?;
        Ok(())
    }

    /// Serializes the full emulator state into a byte vector.
    pub fn save_state(&mut self) -> Result<Vec<u8>, emu_utils::WriteError> {
        use emu_utils::WriteSavestate as _;

        let mut output = Vec::new();
        emu_utils::PersistentWriteSavestate::new(&mut output).store(self)?;
        Ok(output)
    }
}

impl NDS {
    /// Master clock frequency in Hz (33.513982 MHz).
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
        NDS { arm7: ARM::new(&mut hw, direct_boot), arm9: ARM::new(&mut hw, direct_boot), hw }
    }

    #[inline]
    pub fn set_audio_volume(&mut self, volume_percent: f32) {
        self.hw.set_audio_volume(volume_percent);
    }

    /// Runs both CPUs until the GPU signals that a full frame has been rendered.
    ///
    /// Each iteration advances in ≤30-cycle slices to limit desync between
    /// ARM7 and ARM9.  When the 3-D engine stalls the bus the scheduler is
    /// clocked directly and both CPUs are re-synced to avoid starvation.
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
        self.hw.render_palettes(extended, slot, palette, engine, graphics_type)
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
        self.hw.render_tiles(engine, graphics_type, extended, bitmap, bpp8, slot, palette, offset)
    }

    #[inline]
    pub fn render_bank(&self, bank: usize, ignore_alpha: bool) -> (Vec<u16>, usize, usize) {
        self.hw.render_bank(ignore_alpha, bank)
    }

    /// Convenience constructor: loads BIOS / firmware / ROM from the filesystem
    /// and returns a ready-to-run [`NDS`].
    ///
    /// Falls back to the bundled free BIOS / firmware when paths are `None`.
    /// The save file is created automatically next to the ROM if absent.
    pub fn load_rom(
        bios7_path: Option<&Path>,
        bios9_path: Option<&Path>,
        firmware_path: Option<&Path>,
        rom_path: &Path,
        audio_volume: f32,
    ) -> Self {
        let save_file_path = rom_path.with_extension("sav");

        if !save_file_path.exists() {
            info!(
                target: "nds_core::savedata",
                "Save file not found, creating new one at {}",
                save_file_path.display()
            );
        }

        // NOTE: Do not use `truncate`, the .sav file gets corrupted and won't load.
        #[allow(clippy::suspicious_open_options)]
        let save_file =
            OpenOptions::new().read(true).write(true).create(true).open(&save_file_path).unwrap();

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

                    OpenOptions::new().read(true).write(true).open(path).unwrap()
                }
            }
        } else {
            let firmware_path = std::env::temp_dir().join("freebios_firmware.bin");

            if !firmware_path.exists() {
                fs::write(&firmware_path, free_bios::firmware::FIRMWARE_DS).unwrap();
            }

            OpenOptions::new().read(true).write(true).open(&firmware_path).unwrap()
        };

        let mut nds = NDS::new(bios7, bios9, firmware_file, fs::read(rom_path).unwrap(), save_file);
        nds.set_audio_volume(audio_volume);
        nds
    }
}
