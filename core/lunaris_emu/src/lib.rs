// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
#![allow(clippy::missing_const_for_fn)]
mod bios;
mod cpu;
mod dma;
mod emulator;
mod error;
mod firmware;
mod spi;

pub use emulator::{Emulator, emu_config::Config};
