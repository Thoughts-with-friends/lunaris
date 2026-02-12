// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
mod bios;
mod cartridge;
mod cpu;
mod dma;
mod emulator;
mod error;
mod firmware;
mod interrupts;
mod ipc;
mod rtc;
mod spi;
mod timers;
mod touchscreen;
mod wifi;

pub use emulator::{Emulator, emu_config::Config};
