// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! emulator.hpp
//!
use std::path::Path;

use ds_cartridge::CartridgeError;

use crate::{
    emulator::{Emulator, emu_config::BiosMem},
    error::EmuError,
};

impl Emulator {
    // Initialize emulator subsystems.
    // remove pub fn init(&mut self) -> i32

    /// Loads the BIOS and firmware dumps named in `config` (a missing
    /// firmware file falls back to the built-in image).
    ///
    /// ```text
    /// from: frontend, optionally, before load_rom()
    /// to:   arm9_bios / arm7_bios, spi.init(firmware) -> SPI flash and
    ///       the user settings patch
    /// ```
    /// # Errors
    pub fn load_firmware(&mut self) -> Result<(), EmuError> {
        let bin = std::fs::read(&self.config.arm9_bios_path)
            .map_err(EmuError::read(&self.config.arm9_bios_path))?;
        self.arm9_bios = BiosMem::User(bin);

        let bin = std::fs::read(&self.config.arm7_bios_path)
            .map_err(EmuError::read(&self.config.arm7_bios_path))?;
        self.arm7_bios = BiosMem::User(bin);

        let image = read_firmware_image(&self.config.firmware_path)?;
        self.spi.init(&image);
        Ok(())
    }

    /// Load ARM7 BIOS.
    pub fn load_bios7(&mut self, bios: &[u8]) {
        self.arm7_bios = BiosMem::User(bios.to_owned());
    }

    /// Load ARM9 BIOS.
    pub fn load_bios9(&mut self, bios: &[u8]) {
        self.arm9_bios = BiosMem::User(bios.to_owned());
    }

    /// Load firmware image.
    pub fn load_firmware_image(&mut self, firmware: &[u8]) {
        self.spi.init(firmware);
    }

    /// Load save database by name.
    /// # Errors
    pub fn load_save_database(&mut self, name: &Path) -> Result<(), CartridgeError> {
        self.cart.load_database(name)
    }

    /// Loads a `.nds` file and powers the console on.
    ///
    /// ```text
    /// from: frontend (gui/egui run_emulator, gui/tauri emu_thread::start,
    ///       headless main)
    ///   |
    ///   v
    /// load_rom(path)
    ///   1. cartridge_load_rom()   read ROM, pick the save chip and load
    ///                             the .sav, KEY1-encrypt the secure area
    ///                             (non direct boot)
    ///   2. no firmware yet?       spi.init(built-in or configured image)
    ///   3. power_on()             reset everything, direct_boot()
    ///   |
    ///   v
    /// to:   frontend calls run() every frame
    /// ```
    /// # Errors
    pub fn load_rom<A>(&mut self, rom_path: A) -> Result<(), CartridgeError>
    where
        A: AsRef<Path>,
    {
        let rom_path = rom_path.as_ref();
        self.cartridge_load_rom(rom_path)?;

        if !self.spi.firmware.is_loaded() {
            // Falls back to the bundled firmware image when no (or an
            // unreadable) firmware path is configured.
            match read_firmware_image(&self.config.firmware_path) {
                Ok(image) => self.spi.init(&image),
                Err(_err) => {
                    #[cfg(feature = "log")]
                    log::error!("Failed to load firmware: {_err}");
                }
            }
        }

        #[cfg(feature = "log")]
        log::error!("self.cart.rom bytes: {:08X}", self.cart.rom.len());
        self.power_on();
        Ok(())
    }
}

/// Reads the firmware dump at `path`. If it cannot be opened, the built-in
/// replacement firmware is used; a dump shorter than the flash is an error.
fn read_firmware_image(path: &str) -> Result<Vec<u8>, EmuError> {
    use std::io::Read as _;
    let Ok(mut file) = std::fs::File::open(path) else {
        return Ok(ds_bios::firmware::FIRMWARE_DS.to_vec());
    };
    let mut image = vec![0; ds_peripherals::spi::firmware::Firmware::SIZE];
    file.read_exact(&mut image).map_err(EmuError::read(path))?;
    Ok(image)
}
