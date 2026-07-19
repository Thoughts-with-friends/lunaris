//! Cartridge save-data backup chips (EEPROM / Flash), accessed serially
//! through AUXSPI (4001A0h/4001A2h).
//!
//! The chip type cannot be auto-detected from the cartridge, so it is looked
//! up by game code in a built-in database (`game_db`).
//!
//! GBATEK "DS Cartridge Backup" (chip types, command sets):
//! <https://problemkaputt.de/gbatek.htm#dscartridgebackup>
//!
//! See `docs/design/sav-backup-redesign.md` for the rationale behind
//! [`SaveMem`] replacing the previous session-long `mmap` of the `.sav`
//! file (which held the file locked against import/replace for as long as
//! the emulator ran).

mod eeprom;
mod flash;
mod game_db;
mod ir;
mod no_backup;

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::Header;

use eeprom::{EEPROM, EEPROMLarge, EEPROMNormal, EEPROMSmall};
pub use flash::Flash;
use ir::IrBackup;
use no_backup::NoBackup;

pub trait Backup {
    fn read(&self) -> u8;
    fn write(&mut self, hold: bool, value: u8);

    /// Captures the chip's in-flight SPI protocol state (current
    /// command/instruction, address bytes received so far, write-enable
    /// latch, last-read value) for a savestate.
    ///
    /// This deliberately excludes the chip's persistent memory contents,
    /// which are captured separately via [`Backup::save_bytes`] so they can
    /// be embedded directly in the savestate (see
    /// `docs/design/sav-backup-redesign.md` §4.5) rather than relying on a
    /// long-lived file mapping.
    fn protocol_snapshot(&self) -> BackupProtocolState;

    /// Restores a protocol state captured by [`Backup::protocol_snapshot`].
    ///
    /// A variant mismatch (e.g. a savestate captured under a different
    /// backup chip type) is ignored rather than applied, leaving the chip's
    /// current live state untouched.
    fn restore_protocol_state(&mut self, state: BackupProtocolState);

    /// Returns the chip's persistent memory contents (the `.sav` payload).
    /// `None` for chips with no backing store (e.g. [`NoBackup`]).
    fn save_bytes(&self) -> Option<&[u8]>;

    /// Overwrites the chip's persistent memory contents (import, or restoring
    /// a savestate) and flushes to disk immediately. Input is padded/truncated
    /// to the chip's size the same way a foreign `.sav` file is on load.
    fn set_save_bytes(&mut self, bytes: &[u8]);

    /// Flushes any pending writes to the `.sav` file on disk. A no-op if
    /// nothing changed since the last flush. Called automatically whenever
    /// the SPI chip-select is released (see each chip's `write`
    /// implementation) as well as on emulator shutdown.
    fn flush(&mut self);
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
    /// [`IrBackup`]: the IR device-selector phase plus the inner [`Flash`]
    /// chip's own SPI state, flattened together so a savestate captured
    /// mid-transaction restores both. See `docs/design/ir-nand-foreign-sav-design.md` §3.1.
    Ir {
        phase: ir::IrPhase,
        expecting_selector: bool,
        mode: flash::Mode,
        write_enable: bool,
        value: u8,
    },
}

/// In-memory backing store for a backup chip's persistent contents.
///
/// Replaces a session-long `mmap` of the `.sav` file with a plain `Vec<u8>`
/// that is written back to disk only when the hardware itself would commit
/// (SPI chip-select release), on explicit import, or on shutdown. No file
/// handle or memory mapping is held between those points, so the file is
/// never locked against being read, replaced, or deleted by another
/// process. See `docs/design/sav-backup-redesign.md` §3.1/§4.1.
pub struct SaveMem {
    path: PathBuf,
    buf: Vec<u8>,
    dirty: bool,
}

impl SaveMem {
    /// Loads `path` into memory, preserving any existing contents.
    ///
    /// Save files created by other emulators or real flashcarts do not
    /// always match this database's expected size exactly (different
    /// detection heuristics, footers, truncated dumps, …). A size mismatch
    /// keeps the existing bytes and only pads (with `default_val`) or
    /// truncates to reach `size`, matching how other NDS emulators /
    /// flashcart tools handle foreign save files.
    ///
    /// If `path` does not exist yet, the buffer starts `default_val`-filled
    /// (real EEPROM/Flash chips are `0xFF`-filled when erased from the
    /// factory; GBATEK "erased memory is FFh-filled":
    /// <https://problemkaputt.de/gbatek.htm#gbacartbackupeeprom>) and the
    /// file is **not** created on disk until the first [`SaveMem::flush`],
    /// so games that never write a save don't leave behind an empty
    /// `.sav`.
    fn new(path: PathBuf, default_val: u8, size: usize) -> SaveMem {
        let mut buf = normalize_foreign_save(&fs::read(&path).unwrap_or_default());
        let current_len = buf.len();
        if current_len != size {
            if current_len != 0 {
                warn!(
                    target: "nds_core::savedata",
                    "Save file size (0x{current_len:X}) does not match the expected size \
                     (0x{size:X}); resizing while preserving existing data."
                );
            }
            buf.resize(size, default_val);
        }
        debug!(
            target: "nds_core::savedata",
            "SaveMem::new: path={}, default_val=0x{default_val:X}, size=0x{size:X}, \
             current_len=0x{current_len:X}",
            path.display()
        );
        SaveMem { path, buf, dirty: false }
    }

    /// Loads `path` verbatim (no size padding/truncation), for use by the
    /// firmware serial flash chip whose size is whatever the firmware image
    /// on disk already is, rather than a size looked up from a game
    /// database. `path` must already exist.
    pub(crate) fn open_existing(path: PathBuf) -> io::Result<SaveMem> {
        let buf = fs::read(&path)?;
        Ok(SaveMem { path, buf, dirty: false })
    }

    /// Direct mutable access to the backing buffer, for one-off patching
    /// (e.g. the firmware chip stamping touch-calibration bytes into the
    /// user-settings area at boot). Marks the buffer dirty unconditionally,
    /// since the caller is assumed to be about to mutate it.
    pub(crate) fn bytes_mut(&mut self) -> &mut [u8] {
        self.dirty = true;
        &mut self.buf
    }

    #[inline]
    fn buf_len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        &self.buf
    }

    #[inline]
    fn read(&self, addr: usize) -> u8 {
        // Real chips wrap the address bus rather than exposing memory
        // outside their advertised size; without this an over-long
        // read/write burst indexes out of bounds and panics instead of
        // wrapping to address 0.
        self.buf[addr % self.buf.len().max(1)]
    }

    #[inline]
    fn write(&mut self, addr: usize, value: u8) {
        let len = self.buf.len();
        if len == 0 {
            return;
        }
        self.buf[addr % len] = value;
        self.dirty = true;
    }

    fn set_bytes(&mut self, bytes: &[u8], default_val: u8) {
        let size = self.buf.len();
        self.buf.clear();
        self.buf.extend_from_slice(bytes);
        self.buf.resize(size, default_val);
        self.dirty = true;
        self.flush();
    }

    /// Writes the buffer to disk via a temp-file-then-rename sequence, so a
    /// crash mid-write can never leave a torn `.sav` on disk (a state that
    /// the previous `mmap`-backed implementation could produce, since every
    /// SPI byte mutated the mapped page directly and the OS could flush a
    /// dirty page at any time, mid-transaction).
    pub(crate) fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        if let Err(err) = self.flush_to_disk() {
            warn!(
                target: "nds_core::savedata",
                "Failed to flush save file {}: {err}",
                self.path.display()
            );
            return;
        }
        self.dirty = false;
    }

    fn flush_to_disk(&self) -> io::Result<()> {
        let tmp_path = Self::tmp_path(&self.path);
        fs::write(&tmp_path, &self.buf)?;
        fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }

    fn tmp_path(path: &Path) -> PathBuf {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        PathBuf::from(tmp)
    }
}

/// DeSmuME `.dsv` binary footer, written after the raw save payload
/// (optionally preceded by a human-readable "snip here" text banner, which
/// is *not* needed to detect or strip the footer since the binary trailer
/// alone carries everything required).
///
/// Layout (from DeSmuME `src/mc.h`'s `BackupDeviceFileInfo` +
/// `BackupDeviceFileSaveFooter`, all fields little-endian):
/// `[size: u32][padSize: u32][type: u32][addr_size: u32][mem_size: u32]
/// [version: u32][cookie: 16 bytes = "|-DESMUME SAVE-|"]`
///
/// `size` is the true (unpadded) length of the raw save payload that
/// precedes this footer — reading it directly is more robust than
/// searching for the human-readable banner text, whose exact wording is
/// not load-bearing for DeSmuME itself.
const DESMUME_FOOTER_COOKIE: &[u8; 16] = b"|-DESMUME SAVE-|";
const DESMUME_FOOTER_LEN: usize = 4 * 6 + 16;

/// no$gba backup-media save files begin with this ASCII header. Their body
/// is a different (and undocumented enough to be unreliable) container
/// format, not a raw chip image, so they are deliberately not parsed —
/// only detected, so a clear warning can be logged instead of silently
/// importing garbage.
const NOCASH_HEADER: &[u8] = b"NocashGbaBackupMediaSavDataFile";

/// Strips a DeSmuME `.dsv` footer if present, or flags (and discards) a
/// no$gba save container, returning the raw chip payload other emulators
/// and lunaris itself both expect. A melonDS save (already raw) or any
/// other input passes through unchanged. See
/// `docs/design/ir-nand-foreign-sav-design.md` §2.3/§3.3.
///
/// Called on every file read from disk ([`SaveMem::new`]) and on every
/// explicit [`Backup::set_save_bytes`] import, so both the normal load
/// path and manual "Import Save" both accept foreign formats identically.
pub(crate) fn normalize_foreign_save(bytes: &[u8]) -> Vec<u8> {
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

impl dyn Backup {
    /// Looks up the game's backup chip type/size in [`Self::GAME_DB`] and
    /// constructs the matching [`Backup`] implementation.
    ///
    /// `sram_type` 0 means "no backup memory" and some database entries use
    /// `0xFFFFFFFF` as an "unknown" sentinel; both must be routed to
    /// [`NoBackup`] *before* indexing [`Self::SRAM_SIZES`] (which only has
    /// entries for 0..=9), otherwise they panic instead of falling back.
    ///
    /// `sram_type` 8..=9 are NAND-type saves (embedded in the ROM chip and
    /// accessed via ROM commands, not SPI at all — see melonDS
    /// `CartRetailNAND`). They are not yet implemented; routing them to the
    /// SPI [`Flash`] chip would silently produce a save the game can never
    /// read, so they are routed to [`NoBackup`] with a warning instead. See
    /// `docs/design/sav-backup-redesign.md` §3.4/§4.6.
    ///
    /// If the game code is missing from [`Self::GAME_DB`] entirely, an
    /// existing `.sav` file's size (if any) is used to guess the chip type
    /// before falling back to a generic 512 KiB flash chip, instead of
    /// silently giving the game no save capability at all. See
    /// `docs/design/sav-backup-redesign.md` §4.6.
    pub fn detect_type(header: &Header, save_path: PathBuf) -> Box<dyn Backup> {
        let sram_type = <dyn Backup>::GAME_DB
            .iter()
            .find(|game_info| game_info.game_code == header.game_code)
            .map(|game_info| game_info.sram_type)
            .unwrap_or_else(|| {
                let guessed = Self::guess_sram_type_from_existing_file(&save_path);
                warn!(
                    target: "nds_core::savedata",
                    "Game not found in DB! Guessed sram_type=0x{guessed:X}"
                );
                guessed
            });

        // Game codes starting with ASCII `I` route their flash chip through
        // an intermediary IR MCU (Pokémon HeartGold/SoulSilver, Black/White,
        // …); the first byte of the little-endian `game_code` is the
        // cartridge header's first ASCII game-code character. See
        // `docs/design/ir-nand-foreign-sav-design.md` §2.1/§3.1.
        let is_ir_cart = (header.game_code & 0xFF) as u8 == b'I';

        match sram_type {
            1 => {
                Box::new(EEPROM::<EEPROMSmall>::new(save_path, <dyn Backup>::SRAM_SIZES[sram_type]))
            }
            2..=3 => Box::new(EEPROM::<EEPROMNormal>::new(
                save_path,
                <dyn Backup>::SRAM_SIZES[sram_type],
            )),
            // 128K EEPROM uses a 24-bit (3-byte) address bus, unlike the
            // 16-bit bus shared by the 8K/64K variants above.
            // GBATEK: <https://problemkaputt.de/gbatek.htm#dscartridgebackup>
            4 => {
                Box::new(EEPROM::<EEPROMLarge>::new(save_path, <dyn Backup>::SRAM_SIZES[sram_type]))
            }
            5..=7 => {
                let flash = Flash::new_backup(save_path, <dyn Backup>::SRAM_SIZES[sram_type]);
                if is_ir_cart { Box::new(IrBackup::new(flash)) } else { Box::new(flash) }
            }
            8..=9 => {
                warn!(
                    target: "nds_core::savedata",
                    "Game uses a NAND-type save (sram_type=0x{sram_type:X}), which is not yet \
                     supported; save data will not persist."
                );
                Box::new(NoBackup::new())
            }
            sram_type => {
                warn!(
                    target: "nds_core::savedata",
                    "Game has no backup memory (sram_type=0x{sram_type:X})"
                );
                Box::new(NoBackup::new())
            }
        }
    }

    /// Guesses a `sram_type` index (into [`Self::SRAM_SIZES`]) from the size
    /// of an already-existing save file at `save_path`, for games missing
    /// from [`Self::GAME_DB`]. Falls back to a generic 512 KiB flash chip
    /// (`sram_type` 6) — the most common retail save type — if no save file
    /// exists yet or its size doesn't match any known chip.
    fn guess_sram_type_from_existing_file(save_path: &Path) -> usize {
        const GENERIC_FLASH_SRAM_TYPE: usize = 6;
        let Ok(len) = fs::metadata(save_path).map(|meta| meta.len() as usize) else {
            return GENERIC_FLASH_SRAM_TYPE;
        };
        <dyn Backup>::SRAM_SIZES
            .iter()
            .position(|&size| size == len)
            .filter(|&sram_type| sram_type != 0)
            .unwrap_or(GENERIC_FLASH_SRAM_TYPE)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("lunaris_backup_test_{}_{}", std::process::id(), name))
    }

    #[test]
    fn savemem_extends_smaller_file_without_wiping_existing_data() {
        let existing = vec![0x42u8; 0x100];
        let path = temp_path("extend.bin");
        fs::write(&path, &existing).unwrap();

        let mem = SaveMem::new(path.clone(), 0xFF, 0x200);

        assert_eq!(&mem.buf[..0x100], existing.as_slice());
        assert!(mem.buf[0x100..].iter().all(|&b| b == 0xFF));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn savemem_truncates_larger_file_preserving_prefix() {
        let mut existing = vec![0xAAu8; 0x200];
        existing[..4].copy_from_slice(b"SAVE");
        let path = temp_path("truncate.bin");
        fs::write(&path, &existing).unwrap();

        let mem = SaveMem::new(path.clone(), 0x00, 0x100);

        assert_eq!(mem.buf.len(), 0x100);
        assert_eq!(&mem.buf[..4], b"SAVE");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn savemem_matching_size_is_untouched() {
        let existing = vec![0x7Eu8; 0x80];
        let path = temp_path("matching.bin");
        fs::write(&path, &existing).unwrap();

        let mem = SaveMem::new(path.clone(), 0x00, 0x80);

        assert_eq!(&mem.buf[..], existing.as_slice());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn savemem_does_not_create_file_before_first_flush() {
        let path = temp_path("no_create.bin");
        let _ = fs::remove_file(&path);

        let _mem = SaveMem::new(path.clone(), 0xFF, 0x80);

        assert!(!path.exists());
    }

    #[test]
    fn savemem_flush_writes_exact_bytes_and_no_tmp_left_behind() {
        let path = temp_path("flush.bin");
        let _ = fs::remove_file(&path);
        let mut mem = SaveMem::new(path.clone(), 0xFF, 0x10);

        mem.write(0, 0xAB);
        mem.flush();

        assert_eq!(fs::read(&path).unwrap()[0], 0xAB);
        assert!(!SaveMem::tmp_path(&path).exists());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn savemem_read_wraps_out_of_bounds_address() {
        let path = temp_path("wrap.bin");
        let _ = fs::remove_file(&path);
        let mut mem = SaveMem::new(path.clone(), 0x11, 0x4);
        mem.write(0, 0x99);

        // addr = size (one past the end) must wrap to address 0, not panic.
        assert_eq!(mem.read(4), 0x99);
        let _ = fs::remove_file(&path);
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

    /// Builds a synthetic DeSmuME `.dsv` fixture: `raw` save bytes, an
    /// optional banner comment, then the 40-byte binary footer (5x u32
    /// `BackupDeviceFileInfo` + u32 version + 16-byte cookie) per DeSmuME
    /// `src/mc.h`.
    fn desmume_fixture(raw: &[u8], banner: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(raw);
        out.extend_from_slice(banner);
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes()); // size
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes()); // padSize
        out.extend_from_slice(&2u32.to_le_bytes()); // type (arbitrary)
        out.extend_from_slice(&2u32.to_le_bytes()); // addr_size (arbitrary)
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes()); // mem_size
        out.extend_from_slice(&0u32.to_le_bytes()); // version
        out.extend_from_slice(DESMUME_FOOTER_COOKIE);
        out
    }

    #[test]
    fn normalize_strips_desmume_footer_with_banner() {
        let raw = vec![0x5Au8; 0x2000];
        let banner =
            b"|<--Snip above here to create a raw sav by excluding this DeSmuME savedata footer:";
        let fixture = desmume_fixture(&raw, banner);

        assert_eq!(normalize_foreign_save(&fixture), raw);
    }

    #[test]
    fn normalize_strips_desmume_footer_without_banner() {
        // The footer is self-describing (carries its own `size` field), so
        // stripping must not depend on the banner text being present.
        let raw = vec![0x33u8; 0x200];
        let fixture = desmume_fixture(&raw, b"");

        assert_eq!(normalize_foreign_save(&fixture), raw);
    }

    #[test]
    fn normalize_leaves_raw_melonds_style_save_unchanged() {
        let raw = vec![0xFFu8; 0x8_0000];
        assert_eq!(normalize_foreign_save(&raw), raw);
    }

    #[test]
    fn normalize_treats_nocash_header_as_absent() {
        let mut fixture = NOCASH_HEADER.to_vec();
        fixture.extend_from_slice(&[0u8; 64]);
        assert_eq!(normalize_foreign_save(&fixture), Vec::<u8>::new());
    }

    #[test]
    fn normalize_ignores_data_too_short_for_a_footer() {
        // Shorter than the 40-byte footer itself: must not panic on the
        // length subtraction and must pass through unchanged.
        let short = vec![1, 2, 3];
        assert_eq!(normalize_foreign_save(&short), short);
    }
}
