// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! emulator.hpp
//!
use std::path::Path;

use crate::cartridge::CartridgeError;
use crate::emulator::Emulator;
use crate::emulator::emu_config::BiosMem;
use crate::error::{EmuError, FailedReadFileSnafu};
use snafu::ResultExt as _;

impl Emulator {
    /* ===== load (public) ===== */

    /// Initialize emulator subsystems.
    pub fn init(&mut self) -> i32 {
        // self.arm9.set_cp15(&self.arm9_cp15);
        // self.fifo7.receive_queue = &self.fifo7_queue;
        // self.fifo7.send_queue = &self.fifo9_queue;

        // self.fifo9.receive_queue = &self.fifo9_queue;
        // self.fifo9.send_queue = &self.fifo7_queue;
        0
    }

    /// Load firmware from internal source.
    pub fn load_firmware(&mut self) -> Result<(), EmuError> {
        let bin =
            std::fs::read(&self.config.arm9_bios_path).with_context(|_| FailedReadFileSnafu {
                path: &self.config.arm9_bios_path,
            })?;
        self.arm9_bios = BiosMem::User(bin);

        let bin =
            std::fs::read(&self.config.arm7_bios_path).with_context(|_| FailedReadFileSnafu {
                path: &self.config.arm9_bios_path,
            })?;
        self.arm7_bios = BiosMem::User(bin);

        self.spi.init(&self.config.firmware_path)
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
        let _ = firmware;
        unimplemented!("It is not used in C++.");
        // self.spi.init_data(BiosMem::User(firmware.to_owned()));
    }

    /// Load save database by name.
    pub fn load_save_database(&mut self, name: &Path) -> Result<(), CartridgeError> {
        self.cart.load_database(name)
    }

    /// Load a ROM file.
    pub fn load_rom(&mut self, rom_path: &Path) -> Result<(), CartridgeError> {
        self.cartridge_load_rom(rom_path)?;
        self.power_on();
        Ok(())
    }
}
