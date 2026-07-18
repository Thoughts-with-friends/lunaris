//! Savestate file container: `LNST` header + zstd-compressed payload.
//!
//! [`nds_core::nds::NDS::save_state`] returns a raw, uncompressed byte
//! buffer that (as of the P1 core changes) no longer embeds the ROM/BIOS
//! bytes but still contains a few MB of RAM/VRAM/registers. This module
//! wraps that buffer in a small header (magic, format version, ROM
//! fingerprint) and zstd-compresses it before it ever touches disk, and
//! performs the inverse on load.
//!
//! Putting the ROM fingerprint in the header (rather than inside the
//! `emu_utils`-serialized payload) lets [`load_from_file`] reject a
//! savestate that belongs to a different ROM *before* any live emulator
//! state is mutated, and without needing to decompress the payload first.
//!
//! See `docs/design/savestate-and-video-design.md` §1. Shared verbatim
//! between front ends per `docs/design/egui-migration-design.md` §3.2.

use std::{fmt, fs, io, path::Path};

use nds_core::{
    emu_utils,
    nds::{NDS, RomFingerprint},
};

/// Container magic, checked at the start of every file written by
/// [`save_to_file`]. Files lacking it are assumed to be legacy (pre-P1)
/// raw, uncompressed `NDS::save_state()` dumps and are loaded as-is.
const MAGIC: &[u8; 4] = b"LNST";
/// Savestate container format version. Bump when the header layout or the
/// meaning of the compressed payload changes incompatibly.
const VERSION: u32 = 2;
const HEADER_LEN: usize = 4 + 4 + RomFingerprint::ENCODED_LEN;
/// zstd compression level: favors fast save/load over maximum ratio, since
/// this runs on the UI thread during gameplay.
const ZSTD_LEVEL: i32 = 3;

#[derive(Debug)]
pub enum SaveStateError {
    Io(io::Error),
    Write(emu_utils::WriteError),
    Read(emu_utils::ReadError),
    /// The file's ROM fingerprint doesn't match the ROM currently loaded.
    WrongRom {
        expected: RomFingerprint,
        found: RomFingerprint,
    },
}

impl fmt::Display for SaveStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveStateError::Io(e) => write!(f, "I/O error: {e}"),
            SaveStateError::Write(e) => write!(f, "{e}"),
            SaveStateError::Read(e) => write!(f, "{e}"),
            SaveStateError::WrongRom { expected, found } => write!(
                f,
                "savestate belongs to a different ROM (loaded game code {:08X}, savestate game code {:08X})",
                expected.game_code, found.game_code
            ),
        }
    }
}

/// Serializes `nds`, compresses the result, and writes it to `path` behind
/// the `LNST` v2 container header. Creates parent directories as needed.
pub fn save_to_file(nds: &mut NDS, path: &Path) -> Result<(), SaveStateError> {
    let raw = nds.save_state().map_err(SaveStateError::Write)?;
    let compressed = zstd::stream::encode_all(&raw[..], ZSTD_LEVEL).map_err(SaveStateError::Io)?;

    let mut file_bytes = Vec::with_capacity(HEADER_LEN + compressed.len());
    file_bytes.extend_from_slice(MAGIC);
    file_bytes.extend_from_slice(&VERSION.to_le_bytes());
    file_bytes.extend_from_slice(&nds.rom_fingerprint().to_bytes());
    file_bytes.extend_from_slice(&compressed);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(SaveStateError::Io)?;
    }
    fs::write(path, file_bytes).map_err(SaveStateError::Io)
}

/// Reads `path`, verifies it belongs to the ROM currently loaded in `nds`,
/// decompresses it, and applies it via [`NDS::load_state`].
///
/// The ROM fingerprint is checked before any live state is touched, so a
/// mismatched savestate is rejected cleanly instead of corrupting the
/// running session. Files without the `LNST` magic are treated as legacy
/// (pre-P1) raw savestates and loaded directly, without a fingerprint check.
pub fn load_from_file(nds: &mut NDS, path: &Path) -> Result<(), SaveStateError> {
    let file_bytes = fs::read(path).map_err(SaveStateError::Io)?;

    if file_bytes.len() < HEADER_LEN || &file_bytes[0..4] != MAGIC {
        return nds.load_state(&file_bytes).map_err(SaveStateError::Read);
    }

    let version = u32::from_le_bytes(file_bytes[4..8].try_into().unwrap());
    if version != VERSION {
        nds_core::log::warn!(
            target: "nds_core::savedata",
            "Savestate format version {version} (expected {VERSION}); attempting to load anyway"
        );
    }

    let stored_fingerprint_bytes: [u8; RomFingerprint::ENCODED_LEN] =
        file_bytes[8..HEADER_LEN].try_into().unwrap();
    let stored_fingerprint = RomFingerprint::from_bytes(&stored_fingerprint_bytes);
    let current_fingerprint = nds.rom_fingerprint();
    if stored_fingerprint != current_fingerprint {
        return Err(SaveStateError::WrongRom {
            expected: current_fingerprint,
            found: stored_fingerprint,
        });
    }

    let raw = zstd::stream::decode_all(&file_bytes[HEADER_LEN..]).map_err(SaveStateError::Io)?;
    nds.load_state(&raw).map_err(SaveStateError::Read)
}

/// Builds the on-disk path for a numbered savestate slot inside `dir`.
pub fn slot_path(dir: &Path, slot: usize) -> std::path::PathBuf {
    dir.join(format!("state_{slot}.bin"))
}
