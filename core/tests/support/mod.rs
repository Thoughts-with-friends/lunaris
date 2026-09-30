//! Shared infrastructure for the specification tests under `core/tests/`.
//!
//! Every test file in `core/tests/` mirrors one file in `core/src/` and is
//! mounted *as a child module of that source file* (see the
//! `#[cfg(test)] #[path = "…"] mod spec;` line at the bottom of each source
//! file). That gives the tests the same visibility as the code they check, so
//! they can poke private registers and state directly instead of going
//! through the whole emulator.
//!
//! Each test both **asserts** a GBATEK fact and **writes** a human-readable
//! report under `core/tests/dist/`, mirroring the source tree:
//!
//! ```text
//! core/src/hw/cartridge/header.rs  ──►  core/tests/hw/cartridge/header.rs
//!                                          │ (cargo test)
//!                                          ▼
//!                                  core/tests/dist/hw/cartridge/header.md
//! ```
//!
//! Modules:
//! - [`md`]   – Markdown document builder (tables, hexdumps, bitfield boxes)
//! - [`png`]  – tiny RGBA canvas with lines, triangles, grids and a 3×5 font
//! - [`rom`]  – real-ROM loader (`LUNARIS_TEST_ROM`) with a synthetic fallback
//! - [`boot`] – boots an [`crate::NDS`] headlessly and runs it to a predicate
//! - [`spec`] – ✅/❌ ledger turning GBATEK statements into assertions

#![allow(dead_code)]

pub mod boot;
pub mod gx;
pub mod md;
pub mod png;
pub mod rom;
pub mod spec;

use std::path::PathBuf;

/// `core/tests/dist`, resolved from the crate manifest so it does not depend
/// on the working directory VS Code / cargo happens to use.
pub fn dist_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("dist")
}

/// Absolute path of a report file, creating parent directories on demand.
///
/// `rel` uses `/` separators and mirrors the source layout, e.g.
/// `"hw/gpu/engine3d/rendering/wireframe.png"`.
pub fn dist_path(rel: &str) -> PathBuf {
    let path = dist_dir().join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    path
}

/// A per-test scratch directory under the OS temp dir (firmware copies and
/// throw-away `.sav` files go here so real saves are never touched).
pub fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("lunaris-spec").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// GBATEK CRC-16 (poly A001h reflected, init FFFFh) used by the cartridge
/// header, the secure area, the banner and the firmware user settings.
///
/// GBATEK "DS Cartridge Header" (CRC16 at 15Eh) and "BIOS CRC16":
/// <https://problemkaputt.de/gbatek.htm#biosmiscfunctions>
pub fn crc16(init: u16, data: &[u8]) -> u16 {
    const VALS: [u16; 8] = [0xC0C1, 0xC181, 0xC301, 0xC601, 0xCC01, 0xD801, 0xF001, 0xA001];
    let mut crc = init as u32;
    for &byte in data {
        crc ^= byte as u32;
        for (i, &val) in VALS.iter().enumerate() {
            let carry = crc & 1 != 0;
            crc >>= 1;
            if carry {
                crc ^= (val as u32) << (7 - i);
            }
        }
    }
    crc as u16
}

/// Little-endian readers used by all ROM-structure walkers.
pub fn le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

pub fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Printable ASCII rendering of a byte string (`.` for anything else).
pub fn ascii(b: &[u8]) -> String {
    b.iter().map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '.' }).collect()
}

/// Expands a 5-bit colour channel to 8 bits exactly like the GUI does.
pub const fn expand5(c: u16) -> u8 {
    (((c & 0x1F) << 3) | ((c & 0x1F) >> 2)) as u8
}

/// BGR555 (DS palette / framebuffer format) → RGBA8.
pub const fn bgr555_to_rgba(c: u16) -> [u8; 4] {
    [expand5(c), expand5(c >> 5), expand5(c >> 10), 0xFF]
}
