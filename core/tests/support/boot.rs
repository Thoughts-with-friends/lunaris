//! Headless boot helper: builds an [`NDS`] from a [`TestRom`] with the free
//! BIOS/firmware images, private scratch save/firmware files, and runs it
//! frame by frame until a predicate holds.

use super::{rom::TestRom, scratch_dir};
use crate::NDS;

/// Boots `rom` in direct-boot mode. `name` isolates the scratch directory
/// (firmware copy + `.sav`) so parallel tests never share files.
pub fn boot(rom: &TestRom, name: &str) -> NDS {
    let dir = scratch_dir(name);
    let firmware = dir.join("firmware.bin");
    std::fs::write(&firmware, free_bios::firmware::FIRMWARE_DS).unwrap();
    let save = dir.join("game.sav");
    let _ = std::fs::remove_file(&save);
    NDS::new(
        free_bios::arm7::BIOS_ARM7_BIN.to_vec(),
        free_bios::arm9::BIOS_ARM9_BIN.to_vec(),
        firmware,
        rom.bytes.clone(),
        save,
    )
}

/// Runs frames until `pred` returns `true` (checked after every frame) or
/// `max_frames` have elapsed. Returns the number of frames run and whether
/// the predicate was satisfied.
pub fn run_until(
    nds: &mut NDS,
    max_frames: usize,
    mut pred: impl FnMut(&NDS) -> bool,
) -> (usize, bool) {
    for frame in 1..=max_frames {
        nds.emulate_frame();
        if pred(nds) {
            return (frame, true);
        }
    }
    (max_frames, false)
}

/// Frame budget for "boot until" searches. Overridable with
/// `LUNARIS_TEST_FRAMES` because a debug build runs far slower than release.
pub fn frame_budget(default: usize) -> usize {
    std::env::var("LUNARIS_TEST_FRAMES").ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// A freshly constructed synthetic-ROM machine for register-level tests.
/// No frame is run: the test drives I/O ports itself through
/// `hw_mut().arm9_write(..)` / `arm7_write(..)`, exactly like game code does,
/// and advances time with `hw.clock_until(cycle)`.
pub fn io_machine(name: &str) -> NDS {
    boot(super::rom::synthetic(), name)
}
