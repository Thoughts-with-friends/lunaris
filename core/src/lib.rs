//! Core emulation library for the Nintendo DS.
//!
//! Exposes [`NDS`] as the top-level entry point.
//! Internal modules:
//! - [`arm`]  – dual ARM7TDMI / ARM946E-S CPU cores
//! - [`hw`]   – all hardware peripherals (GPU, SPU, DMA, timers, cartridge …)
#![cfg_attr(feature = "nightly", feature(core_intrinsics))]
#![expect(
    clippy::enum_variant_names,
    clippy::module_inception,
    clippy::too_many_arguments,
    clippy::upper_case_acronyms
)]

#[cfg(feature = "nightly")]
use core::intrinsics::{likely, unlikely};

#[cfg(not(feature = "nightly"))]
use likely_stable::{likely, unlikely};

#[macro_use]
pub extern crate log;

use num_traits as num;
pub use simplelog;
// Re-exported so downstream crates can name the error types returned by
// `NDS::save_state`/`NDS::load_state` without taking their own dependency
// on this git crate (and risking a version/rev mismatch).
pub use emu_utils;

mod arm;
mod hw;

pub mod nds;
pub use nds::NDS;

mod macros;

pub type CheatMap = Vec<ArCode>;

/// A single Action Replay style cheat code.
///
/// `code` holds the raw instruction stream, interpreted two `u32` words
/// (opcode+address, parameter) at a time, exactly as the original AR VM does.
pub struct ArCode {
    pub code: Vec<u32>,
    pub enabled: bool,
}
