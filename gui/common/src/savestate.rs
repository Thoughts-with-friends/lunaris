//! Savestate file container: `LNST` header + zstd-compressed payload.
//!
//! ```text
//!  "LNST" | version u32 | ROM fingerprint (16 bytes) | zstd(core state)
//! ```
//!
//! [`Console::save_state`] returns the core's raw state (no ROM / BIOS, a
//! few MB of RAM, VRAM and registers). The ROM fingerprint sits in the
//! header so [`load_from_file`] rejects a state of another ROM before the
//! running console is touched, without decompressing anything.
//!
//! Version 3 states come from the `ds_emu` core; version 2 and headerless
//! files were written by the previous core and cannot be loaded.

use std::{fmt, fs, io, path::Path};

use crate::console::{Console, RomFingerprint, StateError};

const MAGIC: &[u8; 4] = b"LNST";
/// Bump when the header or the payload's meaning changes incompatibly.
const VERSION: u32 = 3;
const HEADER_LEN: usize = 4 + 4 + RomFingerprint::ENCODED_LEN;
/// Favors fast save/load over ratio (runs on the UI thread).
const ZSTD_LEVEL: i32 = 3;

#[derive(Debug)]
pub enum SaveStateError {
    Io(io::Error),
    Core(StateError),
    /// Not a version-3 `LNST` file (an older core's state or another file).
    Unsupported {
        version: Option<u32>,
    },
    /// The file's ROM fingerprint doesn't match the ROM currently loaded.
    WrongRom {
        expected: RomFingerprint,
        found: RomFingerprint,
    },
}

impl fmt::Display for SaveStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Core(e) => write!(f, "{e}"),
            Self::Unsupported { version: Some(v) } => write!(
                f,
                "savestate format version {v} is not supported (made by another core version; \
                 expected {VERSION})"
            ),
            Self::Unsupported { version: None } => f.write_str("not a lunaris savestate"),
            Self::WrongRom { expected, found } => write!(
                f,
                "savestate belongs to a different ROM (loaded game code {:08X}, savestate game code {:08X})",
                expected.game_code, found.game_code
            ),
        }
    }
}

impl std::error::Error for SaveStateError {}

/// Saves `console` to `path` (parent directories are created).
///
/// # Errors
/// If the state cannot be encoded or the file cannot be written.
pub fn save_to_file(console: &Console, path: &Path) -> Result<(), SaveStateError> {
    let raw = console.save_state().map_err(SaveStateError::Core)?;
    let compressed = zstd::stream::encode_all(&raw[..], ZSTD_LEVEL).map_err(SaveStateError::Io)?;

    let mut file_bytes = Vec::with_capacity(HEADER_LEN + compressed.len());
    file_bytes.extend_from_slice(MAGIC);
    file_bytes.extend_from_slice(&VERSION.to_le_bytes());
    file_bytes.extend_from_slice(&console.rom_fingerprint().to_bytes());
    file_bytes.extend_from_slice(&compressed);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(SaveStateError::Io)?;
    }
    fs::write(path, file_bytes).map_err(SaveStateError::Io)
}

/// Loads the state at `path` into `console`, after checking it was made
/// with the loaded ROM. On any error the console is unchanged.
///
/// # Errors
/// If the file cannot be read, is not a state of this core, or belongs to
/// another ROM.
pub fn load_from_file(console: &mut Console, path: &Path) -> Result<(), SaveStateError> {
    let file_bytes = fs::read(path).map_err(SaveStateError::Io)?;
    let Some((header, payload)) = file_bytes.split_first_chunk::<HEADER_LEN>() else {
        return Err(SaveStateError::Unsupported { version: None });
    };
    if &header[0..4] != MAGIC {
        return Err(SaveStateError::Unsupported { version: None });
    }
    let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if version != VERSION {
        return Err(SaveStateError::Unsupported { version: Some(version) });
    }

    let mut stored = [0_u8; RomFingerprint::ENCODED_LEN];
    stored.copy_from_slice(&header[8..]);
    let found = RomFingerprint::from_bytes(&stored);
    let expected = console.rom_fingerprint();
    if found != expected {
        return Err(SaveStateError::WrongRom { expected, found });
    }

    let raw = zstd::stream::decode_all(payload).map_err(SaveStateError::Io)?;
    console.load_state(&raw).map_err(SaveStateError::Core)
}

/// Builds the on-disk path for a numbered savestate slot inside `dir`.
pub fn slot_path(dir: &Path, slot: usize) -> std::path::PathBuf {
    dir.join(format!("state_{slot}.bin"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rejects() {
        let dir = std::env::temp_dir().join(format!("lunaris_state_test_{}", std::process::id()));
        let path = dir.join("state_1.bin");
        let mut console = Console::placeholder();
        console.emulator_mut().main_ram[0x40] = 0x77;
        save_to_file(&console, &path).unwrap();

        console.emulator_mut().main_ram[0x40] = 0;
        load_from_file(&mut console, &path).unwrap();
        assert_eq!(console.emulator().main_ram[0x40], 0x77);

        let mut bytes = fs::read(&path).unwrap();
        bytes[4] = 2;
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            load_from_file(&mut console, &path),
            Err(SaveStateError::Unsupported { version: Some(2) })
        ));
        fs::write(&path, b"raw").unwrap();
        assert!(matches!(
            load_from_file(&mut console, &path),
            Err(SaveStateError::Unsupported { version: None })
        ));
        let _ = fs::remove_dir_all(dir);
    }
}
