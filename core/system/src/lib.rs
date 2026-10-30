// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
#![allow(clippy::missing_const_for_fn)]
mod bios;
mod cpu;
mod dma;
mod emulator;
mod error;
pub mod slot2;

pub use ds_cartridge::{RomFingerprint, normalize_foreign_save};
pub use ds_cpu::arm_cpu::CpuType;
pub use ds_peripherals::wifi::MpLink;
pub use emulator::{Emulator, cheats::parse_ar_code, emu_config::Config, state::StateError};
