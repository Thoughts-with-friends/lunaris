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

const DESMUME_FOOTER_COOKIE: &[u8; 16] = b"|-DESMUME SAVE-|";
const DESMUME_FOOTER_LEN: usize = 4 * 6 + 16;
const NOCASH_HEADER: &[u8] = b"NocashGbaBackupMediaSavDataFile";

pub fn normalize_foreign_save(bytes: &[u8]) -> Vec<u8> {
    if let Some(raw) = strip_desmume_footer(bytes) {
        info!(
            target: "nds_core::savedata",
            "DeSmuME .dsv footer detected; stripped to {} raw bytes",
            raw.len()
        );
        return raw.to_vec();
    }
    if bytes.starts_with(NOCASH_HEADER) {
        warn!(
            target: "nds_core::savedata",
            "no$gba save format detected; this format is not supported, treating as absent"
        );
        return Vec::new();
    }
    bytes.to_vec()
}

fn strip_desmume_footer(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < DESMUME_FOOTER_LEN {
        return None;
    }
    let footer_start = bytes.len() - DESMUME_FOOTER_LEN;
    if &bytes[bytes.len() - 16..] != DESMUME_FOOTER_COOKIE {
        return None;
    }
    let raw_size =
        u32::from_le_bytes(bytes[footer_start..footer_start + 4].try_into().ok()?) as usize;
    // The declared raw size must fit within the region preceding the
    // footer (which may also contain the human-readable banner text); a
    // value that doesn't is either a corrupt footer or an unlucky
    // coincidental cookie match in unrelated binary data, so bail out
    // rather than risk truncating to a bogus length.
    if raw_size > footer_start { None } else { Some(&bytes[..raw_size]) }
}
