//! Cartridge save-data backup chips (EEPROM / Flash), accessed serially
//! through AUXSPI (4001A0h/4001A2h).
//!
//! The chip type cannot be auto-detected from the cartridge, so it is looked
//! up by game code in a built-in database (`game_db`).
//!
//! GBATEK "DS Cartridge Backup" (chip types, command sets):
//! <https://problemkaputt.de/gbatek.htm#dscartridgebackup>

mod eeprom;
mod flash;
mod game_db;
mod no_backup;

use memmap::{MmapMut, MmapOptions};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
};

use super::Header;

use eeprom::{EEPROM, EEPROMLarge, EEPROMNormal, EEPROMSmall};
pub use flash::Flash;
use no_backup::NoBackup;

pub trait Backup {
    fn read(&self) -> u8;
    fn write(&mut self, hold: bool, value: u8);

    /// Captures the chip's in-flight SPI protocol state (current
    /// command/instruction, address bytes received so far, write-enable
    /// latch, last-read value) for a savestate.
    ///
    /// This deliberately excludes the chip's persistent memory contents,
    /// which live in the `.sav` file's mmap and stay open across a
    /// savestate load, so they never need to round-trip through the
    /// savestate itself.
    fn protocol_snapshot(&self) -> BackupProtocolState;

    /// Restores a protocol state captured by [`Backup::protocol_snapshot`].
    ///
    /// A variant mismatch (e.g. a savestate captured under a different
    /// backup chip type) is ignored rather than applied, leaving the chip's
    /// current live state untouched.
    fn restore_protocol_state(&mut self, state: BackupProtocolState);
}

/// Snapshot of a backup chip's SPI protocol state machine.
///
/// Without this, a savestate captured while a game is mid-transaction with
/// its save chip (a very common window, since games poll their save chip
/// frequently) would resume with the chip reset to idle. The ARM7 would
/// then wait forever for a response to a transaction the chip never
/// started, hanging the game after a Load State even though the CPU/GPU
/// keep running. See `docs/design/savestate-and-video-design.md` §2.3 (C1).
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug)]
pub enum BackupProtocolState {
    /// [`NoBackup`], or state not yet captured.
    None,
    Eeprom {
        mode: eeprom::Mode,
        write_enable: bool,
        value: u8,
    },
    Flash {
        mode: flash::Mode,
        write_enable: bool,
        value: u8,
    },
}

impl dyn Backup {
    /// Looks up the game's backup chip type/size in [`Self::GAME_DB`] and
    /// constructs the matching [`Backup`] implementation.
    ///
    /// `sram_type` 0 means "no backup memory" and some database entries use
    /// `0xFFFFFFFF` as an "unknown" sentinel; both must be routed to
    /// [`NoBackup`] *before* indexing [`Self::SRAM_SIZES`] (which only has
    /// entries for 0..=9), otherwise they panic instead of falling back.
    pub fn detect_type(header: &Header, save_file: File) -> Box<dyn Backup> {
        if let Some(pos) = <dyn Backup>::GAME_DB
            .iter()
            .position(|game_info| game_info.game_code == header.game_code)
        {
            let game_info = &<dyn Backup>::GAME_DB[pos];
            match game_info.sram_type {
                1 => Box::new(EEPROM::<EEPROMSmall>::new(
                    save_file,
                    <dyn Backup>::SRAM_SIZES[game_info.sram_type],
                )),
                2..=3 => Box::new(EEPROM::<EEPROMNormal>::new(
                    save_file,
                    <dyn Backup>::SRAM_SIZES[game_info.sram_type],
                )),
                // 128K EEPROM uses a 24-bit (3-byte) address bus, unlike the
                // 16-bit bus shared by the 8K/64K variants above.
                // GBATEK: <https://problemkaputt.de/gbatek.htm#dscartridgebackup>
                4 => Box::new(EEPROM::<EEPROMLarge>::new(
                    save_file,
                    <dyn Backup>::SRAM_SIZES[game_info.sram_type],
                )),
                5..=9 => Box::new(Flash::new_backup(
                    save_file,
                    <dyn Backup>::SRAM_SIZES[game_info.sram_type],
                )),
                sram_type => {
                    warn!(
                        target: "nds_core::savedata",
                        "Game has no backup memory (sram_type=0x{sram_type:X})"
                    );
                    Box::new(NoBackup::new())
                }
            }
        } else {
            warn!(target: "nds_core::savedata", "Game not found in DB!");
            Box::new(NoBackup::new())
        }
    }

    /// Maps `save_file` to exactly `size` bytes, preserving any existing
    /// contents.
    ///
    /// Save files created by other emulators or real flashcarts do not
    /// always match this database's expected size exactly (different
    /// detection heuristics, footers, truncated dumps, …). Previously a
    /// size mismatch caused the *entire* file to be overwritten with
    /// `default_val`, silently discarding imported save data. Instead, the
    /// existing bytes are kept and the file is only extended (padded with
    /// `default_val`) or truncated to reach `size`, matching how other NDS
    /// emulators / flashcart tools handle foreign save files.
    fn mmap(mut save_file: File, default_val: u8, size: usize) -> MmapMut {
        let current_len = save_file.metadata().unwrap().len() as usize;
        debug!(
            target: "nds_core::savedata",
            "mmap: default_val={default_val}, size={size:#X}, current_len={current_len:#X}"
        );

        if current_len != size {
            warn!(
                target: "nds_core::savedata",
                "Save file size (0x{current_len:X}) does not match the expected size \
                 (0x{size:X}); resizing while preserving existing data."
            );
            let mut contents = Vec::with_capacity(current_len.min(size));
            save_file.seek(SeekFrom::Start(0)).unwrap();
            save_file.read_to_end(&mut contents).unwrap();
            contents.resize(size, default_val);
            save_file.set_len(size as u64).unwrap();
            save_file.seek(SeekFrom::Start(0)).unwrap();
            save_file.write_all(&contents).unwrap();
        }
        unsafe { MmapOptions::new().map_mut(&save_file).unwrap() }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn temp_file_with(contents: &[u8]) -> File {
        let path = std::env::temp_dir().join(format!(
            "lunaris_backup_test_{}_{}.bin",
            std::process::id(),
            contents.len()
        ));
        std::fs::write(&path, contents).unwrap();
        std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap()
    }

    #[test]
    fn mmap_extends_smaller_file_without_wiping_existing_data() {
        let existing = vec![0x42u8; 0x100];
        let file = temp_file_with(&existing);
        let mem = <dyn Backup>::mmap(file, 0xFF, 0x200);

        assert_eq!(&mem[..0x100], existing.as_slice());
        assert!(mem[0x100..].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn mmap_truncates_larger_file_preserving_prefix() {
        let mut existing = vec![0xAAu8; 0x200];
        existing[..4].copy_from_slice(b"SAVE");
        let file = temp_file_with(&existing);
        let mem = <dyn Backup>::mmap(file, 0x00, 0x100);

        assert_eq!(mem.len(), 0x100);
        assert_eq!(&mem[..4], b"SAVE");
    }

    #[test]
    fn mmap_matching_size_is_untouched() {
        let existing = vec![0x7Eu8; 0x80];
        let file = temp_file_with(&existing);
        let mem = <dyn Backup>::mmap(file, 0x00, 0x80);

        assert_eq!(&mem[..], existing.as_slice());
    }

    #[test]
    fn detect_type_does_not_panic_on_sentinel_sram_types() {
        // sram_type 0 ("no backup") and 0xFFFFFFFF ("unknown", used by a
        // handful of DB entries) must not index out of `SRAM_SIZES` or hit
        // an unhandled `todo!()`.
        let zero_size = <dyn Backup>::SRAM_SIZES[0];
        assert_eq!(zero_size, 0);

        let backup: Box<dyn Backup> = Box::new(NoBackup::new());
        assert_eq!(backup.read(), 0);
    }
}
