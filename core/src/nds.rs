use crate::{CheatMap, likely};
use std::path::PathBuf;

use crate::arm::ARM;
use crate::hw::HW;

pub use crate::hw::{
    Engine, GraphicsType, Key, LinkHints, LoopbackTransport, MpFrameKind, MpRecv, MpTransport,
    RomFingerprint,
};

pub const WIDTH: usize = crate::hw::GPU::WIDTH;
pub const HEIGHT: usize = crate::hw::GPU::HEIGHT;

/// Top-level Nintendo DS emulator state.
///
/// Owns both CPU cores and all hardware peripherals.
/// Call [`NDS::emulate_frame`] repeatedly to drive emulation.
#[derive(emu_utils::Savestate)]
#[load(in_place_only)]
pub struct NDS {
    /// ARM7TDMI sub-processor (audio, Wi-Fi, I/O assist).
    arm7: ARM<false>,
    /// ARM946E-S main processor (game code, 3D, main memory).
    arm9: ARM<true>,
    hw: HW,
}

impl NDS {
    /// Deserializes a previously saved state into `self`.
    pub fn load_state(&mut self, state: &[u8]) -> Result<(), emu_utils::ReadError> {
        use emu_utils::ReadSavestate as _;
        let mut reader = emu_utils::PersistentReadSavestate::new(state)
            .map_err(|_| emu_utils::ReadError::InvalidEnum)?;
        reader.load_into(self)?;
        Ok(())
    }

    /// Serializes the full emulator state into a byte vector.
    pub fn save_state(&mut self) -> Result<Vec<u8>, emu_utils::WriteError> {
        use emu_utils::WriteSavestate as _;

        let mut output = Vec::new();
        emu_utils::PersistentWriteSavestate::new(&mut output).store(self)?;
        Ok(output)
    }

    /// Returns a compact fingerprint of the currently loaded ROM, used to
    /// verify a savestate file was captured against the same ROM before
    /// applying it. See `docs/design/savestate-and-video-design.md` §1.3.
    pub fn rom_fingerprint(&self) -> RomFingerprint {
        self.hw.rom_fingerprint()
    }
}

impl NDS {
    /// Master clock frequency in Hz (33.513982 MHz = ARM7 clock; the ARM9
    /// runs at exactly 2× this rate).
    ///
    /// GBATEK "DS Technical Data – Processors":
    /// <https://problemkaputt.de/gbatek.htm#dstechnicaldata>
    pub const CLOCK_RATE: usize = 33513982;

    pub fn new(
        bios7: Vec<u8>,
        bios9: Vec<u8>,
        firmware_path: PathBuf,
        rom: Vec<u8>,
        save_path: PathBuf,
    ) -> Self {
        let direct_boot = true;
        let mut hw = HW::new(bios7, bios9, firmware_path, rom, save_path, direct_boot);
        NDS { arm7: ARM::new(&mut hw, direct_boot), arm9: ARM::new(&mut hw, direct_boot), hw }
    }

    #[inline]
    pub fn set_audio_volume(&mut self, volume_percent: f32) {
        self.hw.set_audio_volume(volume_percent);
    }

    /// Installs (or removes, with `None`) the frontend-supplied MP
    /// transport that carries multiplayer frames to/from other `lunaris`
    /// instances. See `docs/design/design_lan.md` §8.1.
    #[inline]
    pub fn set_mp_transport(&mut self, transport: Option<Box<dyn MpTransport>>) {
        self.hw.set_mp_transport(transport);
    }

    /// Loads RF channel calibration from a firmware Wi-Fi config block
    /// (starting at firmware offset `02Ch`). See
    /// `docs/design/design_lan.md` §7.
    #[inline]
    pub fn load_wifi_firmware_config(&mut self, config: &[u8]) {
        self.hw.load_wifi_firmware_config(config);
    }

    /// Returns `true` if the Wi-Fi hardware currently believes it is
    /// engaged in a multiplayer session (host or client).
    #[inline]
    pub fn wifi_mp_active(&self) -> bool {
        self.hw.wifi_mp_active()
    }

    /// Current adaptive link parameters in effect. UI display use only.
    #[inline]
    pub fn wifi_link_hints(&self) -> LinkHints {
        self.hw.wifi_link_hints()
    }

    /// Diagnostic escape hatch: see [`crate::hw::HW::wifi_write16`]. Used by
    /// `core/examples/mp_loopback.rs` to drive Wi-Fi hardware directly
    /// without a Wi-Fi-capable test ROM.
    #[inline]
    pub fn wifi_write16(&mut self, addr: u32, value: u16) {
        self.hw.wifi_write16(addr, value);
    }

    /// Diagnostic escape hatch: see [`crate::hw::HW::wifi_read16`].
    #[inline]
    pub fn wifi_read16(&mut self, addr: u32) -> u16 {
        self.hw.wifi_read16(addr)
    }

    /// Diagnostic escape hatch: see [`crate::hw::HW::wifi_set_power`].
    #[inline]
    pub fn wifi_set_power(&mut self, enable: bool) {
        self.hw.wifi_set_power(enable);
    }

    /// Diagnostic escape hatch: see [`crate::hw::HW::wifi_debug_tick`].
    #[inline]
    pub fn wifi_debug_tick(&mut self, ticks: u32) {
        self.hw.wifi_debug_tick(ticks);
    }

    /// Imports external cartridge save data (e.g. from another emulator or
    /// a flashcart dump), replacing the current save and flushing it to the
    /// `.sav` file immediately. Best done at the game's title screen, since
    /// the running game may hold a stale in-RAM copy of its save data. See
    /// `docs/design/sav-backup-redesign.md` §4.4.
    pub fn import_save(&mut self, bytes: &[u8]) {
        self.hw.import_save(bytes);
    }

    /// Returns a copy of the current cartridge save data.
    pub fn export_save(&mut self) -> Vec<u8> {
        self.hw.export_save()
    }

    /// Flushes any pending cartridge save-chip writes to the `.sav` file.
    /// Call before dropping [`NDS`] / on emulator shutdown to guarantee a
    /// transaction that never released chip-select is not lost.
    pub fn flush_save(&mut self) {
        self.hw.flush_save();
    }

    /// Test-only: shifts every absolute cycle counter (ARM9/ARM7 CPU cycles,
    /// scheduler cycle, timer start cycles) by `offset`, simulating a long
    /// play session without actually running billions of cycles. ARM9 runs
    /// at 2× the master clock, so its counter is shifted by `2 * offset`.
    /// See `docs/design/savestate-and-video-design.md` §3.4.
    #[cfg(test)]
    pub(crate) fn offset_cycles_for_test(&mut self, offset: usize) {
        self.arm9.offset_cycle_for_test(offset * 2);
        self.arm7.offset_cycle_for_test(offset);
        self.hw.offset_cycles_for_test(offset);
    }

    /// Runs both CPUs until the GPU signals that a full frame has been rendered.
    ///
    /// Each iteration advances in ≤30-cycle slices to limit desync between
    /// ARM7 and ARM9.  When the 3-D engine stalls the bus the scheduler is
    /// clocked directly and both CPUs are re-synced to avoid starvation.
    pub fn emulate_frame(&mut self) {
        while !self.hw.rendered_frame() {
            if likely(!self.hw.gpu.bus_stalled()) {
                let cycle = self.hw.cycle();
                // The max cycle desync was ~30 when the CPUs were running tightly
                let target = std::cmp::min(cycle + 30, self.hw.cycle_at_next_event());

                self.arm9.emulate(&mut self.hw, target * 2);
                self.arm7.emulate(&mut self.hw, target);
                self.hw.clock_until(target);
            } else {
                self.hw.clock_until_event();
                self.arm9.set_cycle(self.hw.cycle() * 2);
                self.arm7.set_cycle(self.hw.cycle());
            }
        }

        if self.hw.enable_cheats {
            self.hw.apply_cheats();
        }
    }

    /// gui api
    #[inline]
    pub fn get_cheat_map(&self) -> &CheatMap {
        &self.hw.cheat_map
    }

    /// gui api
    #[inline]
    pub fn set_cheat_map(&mut self, cheat_map: CheatMap) {
        // 0223_DD34 6008_0180
        // 0223_DD38 309C_1C28
        self.hw.cheat_map = cheat_map;
    }

    #[inline]
    pub fn set_enable_cheats(&mut self, enable: bool) {
        self.hw.enable_cheats = enable;
    }

    #[inline]
    pub fn get_screens(&self) -> [&Vec<u16>; 2] {
        self.hw.gpu.get_screens()
    }

    #[inline]
    pub fn press_key(&mut self, key: Key) {
        self.hw.press_key(key);
    }

    #[inline]
    pub fn release_key(&mut self, key: Key) {
        self.hw.release_key(key);
    }

    #[inline]
    pub fn press_screen(&mut self, x: usize, y: usize) {
        self.hw.press_screen(x, y);
    }

    #[inline]
    pub fn release_screen(&mut self) {
        self.hw.release_screen();
    }

    #[inline]
    pub fn render_palettes(
        &self,
        extended: bool,
        slot: usize,
        palette: usize,
        engine: Engine,
        graphics_type: GraphicsType,
    ) -> (Vec<u16>, usize, usize) {
        self.hw.render_palettes(extended, slot, palette, engine, graphics_type)
    }

    #[inline]
    pub fn render_map(&self, engine: Engine, bg_i: usize) -> (Vec<u16>, usize, usize) {
        self.hw.render_map(engine, bg_i)
    }

    #[inline]
    pub fn render_tiles(
        &self,
        engine: Engine,
        graphics_type: GraphicsType,
        extended: bool,
        bitmap: bool,
        bpp8: bool,
        slot: usize,
        palette: usize,
        offset: usize,
    ) -> (Vec<u16>, usize, usize) {
        self.hw.render_tiles(engine, graphics_type, extended, bitmap, bpp8, slot, palette, offset)
    }

    #[inline]
    pub fn render_bank(&self, bank: usize, ignore_alpha: bool) -> (Vec<u16>, usize, usize) {
        self.hw.render_bank(ignore_alpha, bank)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };

    fn hash_screens(nds: &NDS) -> u64 {
        let mut hasher = DefaultHasher::new();
        for screen in nds.get_screens() {
            screen.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn make_nds() -> NDS {
        let bios7 = free_bios::arm7::BIOS_ARM7_BIN.to_vec();
        let bios9 = free_bios::arm9::BIOS_ARM9_BIN.to_vec();

        let fw_path = std::env::temp_dir().join("lunaris_test_fw.bin");
        if !fw_path.exists() {
            std::fs::write(&fw_path, free_bios::firmware::FIRMWARE_DS).unwrap();
        }

        let save_path = std::env::temp_dir().join("lunaris_test.sav");

        // Binary of the smallest valid NDS file: Only the hex-dump
        // https://imrannazar.com/The-Smallest-NDS-File#:~:text=Final%20binary%3A%20352%20bytes
        let tiny_rom = std::fs::read("../target/tiny_rom.nds").unwrap();
        NDS::new(bios7, bios9, fw_path, tiny_rom, save_path)
    }

    #[ignore = "because we need external test files"]
    #[test]
    fn test_rom_size() {
        // NDS init_mem copies the first 0x170 bytes of the ROM into main memory.
        let tiny_rom = std::fs::read("../target/tiny_rom.nds").unwrap();
        assert_eq!(tiny_rom.len(), 0x170);
    }

    #[ignore = "because we need external test files"]
    #[test]
    fn test_emulate_one_frame() {
        let mut nds = make_nds();
        nds.emulate_frame();
    }

    #[ignore = "because we need external test files"]
    #[test]
    fn test_save_state_non_empty() {
        let mut nds = make_nds();
        nds.emulate_frame();
        let state = nds.save_state().unwrap();
        assert!(!state.is_empty());
    }

    #[ignore = "because we need external test files"]
    #[test]
    fn test_load_state_after_save() {
        let mut nds = make_nds();
        for _ in 0..3 {
            nds.emulate_frame();
        }
        let state = nds.save_state().unwrap();
        nds.load_state(&state).unwrap();
        // Emulate a few more frames to ensure the state is valid and the emulator continues to run.
        for _ in 0..3 {
            nds.emulate_frame();
        }
    }

    #[ignore = "because we need external test files"]
    #[test]
    fn test_load_state_deterministic() {
        let mut nds = make_nds();
        for _ in 0..5 {
            nds.emulate_frame();
        }
        let state = nds.save_state().unwrap();

        nds.load_state(&state).unwrap();
        let state2 = nds.save_state().unwrap();

        nds.load_state(&state).unwrap();
        let state3 = nds.save_state().unwrap();

        assert_eq!(state2, state3, "State should be deterministic after loading the same state.");
    }

    /// Regression test for the emu-utils `usize`-as-`u32` truncation bug
    /// (see `docs/design/savestate-and-video-design.md` §3): without the
    /// `u64` serialization fix on ARM/Scheduler/Timer cycle counters, a
    /// save/load cycle after crossing the 2^32 cycle boundary corrupts
    /// ARM9's cycle counter, which desyncs it from the scheduler and causes
    /// `emulate_frame` to spin effectively forever trying to catch up.
    #[ignore = "because we need external test files"]
    #[test]
    fn test_load_state_after_u32_cycle_overflow() {
        let mut nds = make_nds();
        for _ in 0..2 {
            nds.emulate_frame();
        }

        // Simulate ~2-3 minutes of real play by jumping every absolute cycle
        // counter past the u32 boundary.
        nds.offset_cycles_for_test(0x1_0000_0000);

        for _ in 0..2 {
            nds.emulate_frame();
        }

        let state = nds.save_state().unwrap();

        // Must not hang: pre-fix, ARM9's truncated cycle counter desyncs from
        // the scheduler and this loop never returns.
        nds.load_state(&state).unwrap();
        for _ in 0..3 {
            nds.emulate_frame();
        }

        // Determinism check, mirroring `test_load_state_deterministic`.
        nds.load_state(&state).unwrap();
        let state2 = nds.save_state().unwrap();
        nds.load_state(&state).unwrap();
        let state3 = nds.save_state().unwrap();
        assert_eq!(
            state2, state3,
            "State should be deterministic after loading a state saved past the u32 cycle boundary."
        );
    }

    /// Regression test for both P1 (savestate size) and P2 (post-load
    /// freeze) from `docs/design/savestate-and-video-design.md`, using a
    /// real ~128 MB commercial ROM so its cartridge backup chip (Flash)
    /// receives genuine SPI traffic during boot -- something the tiny
    /// synthetic ROM used by the other tests in this module cannot exercise,
    /// since it has no backup chip.
    ///
    /// - P1: asserts the raw `save_state()` payload is a small fraction of
    ///   the ROM size, proving `Cartridge::rom`/`HW::bios7`/`HW::bios9` are
    ///   no longer serialized.
    /// - P2: saves mid-boot (while the game is actively polling its save
    ///   chip), keeps running past the save point so live state diverges,
    ///   loads the earlier state back, and asserts the displayed screens
    ///   keep changing across subsequent frames. Before the
    ///   `BackupProtocolState` fix, the backup chip's SPI transaction state
    ///   was silently reset on load while the CPU/GXFIFO/scheduler state
    ///   was rewound, leaving the ARM7 waiting forever on a save-chip
    ///   response it would never receive -- the screens would go static
    ///   even though `emulate_frame` keeps returning (FPS keeps counting).
    #[ignore = "because we need a real commercial ROM (../target/test_rom.nds)"]
    #[test]
    fn test_load_state_real_rom_size_and_no_freeze() {
        let bios7 = free_bios::arm7::BIOS_ARM7_BIN.to_vec();
        let bios9 = free_bios::arm9::BIOS_ARM9_BIN.to_vec();

        let fw_path = std::env::temp_dir().join("lunaris_test_fw_real.bin");
        if !fw_path.exists() {
            std::fs::write(&fw_path, free_bios::firmware::FIRMWARE_DS).unwrap();
        }

        let save_path = std::env::temp_dir().join("lunaris_test_real.sav");
        let _ = std::fs::remove_file(&save_path);

        let rom = std::fs::read("../target/test_rom.nds").unwrap();
        let rom_len = rom.len();
        let mut nds = NDS::new(bios7, bios9, fw_path, rom, save_path);

        // Run well into the boot sequence (health & safety screen, save-chip
        // polling, etc.) before saving.
        for _ in 0..300 {
            nds.emulate_frame();
        }

        let state = nds.save_state().unwrap();
        assert!(
            state.len() < rom_len / 10,
            "raw savestate ({} bytes) should be far smaller than the ROM ({} bytes); \
             Cartridge::rom is no longer expected to be serialized",
            state.len(),
            rom_len
        );

        // Diverge live state from the save point.
        for _ in 0..120 {
            nds.emulate_frame();
        }

        nds.load_state(&state).unwrap();

        let mut last_hash = hash_screens(&nds);
        let mut screens_changed = false;
        for _ in 0..180 {
            nds.emulate_frame();
            let hash = hash_screens(&nds);
            if hash != last_hash {
                screens_changed = true;
            }
            last_hash = hash;
        }
        assert!(
            screens_changed,
            "displayed screens never changed in 180 frames after Load State; \
             gameplay appears frozen (see docs/design/savestate-and-video-design.md §2)"
        );
    }

    /// Regression test for `docs/design/sav-backup-redesign.md` §3.1: the
    /// previous `mmap`-backed backup implementation held the `.sav` file
    /// memory-mapped for the entire emulator session, which on Windows
    /// blocks any external rename/delete/truncate of that file with
    /// `ERROR_USER_MAPPED_FILE` -- the reported "save is locked, can't be
    /// imported/replaced" bug. Runs a real commercial ROM (Flash-backed
    /// save chip) well past its save-chip boot polling, then proves the
    /// `.sav` file can be renamed and replaced by another process while the
    /// `NDS` instance is still alive, and that [`NDS::import_save`] /
    /// [`NDS::export_save`] work mid-session.
    #[ignore = "because we need a real commercial ROM (../target/test_rom.nds)"]
    #[test]
    fn test_sav_file_not_locked_during_session() {
        let bios7 = free_bios::arm7::BIOS_ARM7_BIN.to_vec();
        let bios9 = free_bios::arm9::BIOS_ARM9_BIN.to_vec();

        let fw_path = std::env::temp_dir().join("lunaris_test_fw_lock.bin");
        if !fw_path.exists() {
            std::fs::write(&fw_path, free_bios::firmware::FIRMWARE_DS).unwrap();
        }

        // Start from a real, pre-populated save file (as an actual user
        // would have) rather than a fresh/empty one: with the new
        // write-on-flush design a chip that is only ever *read* from during
        // boot never creates a file at all, which would make this test
        // pass for the wrong reason (no file exists, so nothing was locked).
        let save_path = std::env::temp_dir().join("lunaris_test_lock.sav");
        std::fs::copy("../target/test_rom.sav", &save_path).unwrap();

        let rom = std::fs::read("../target/test_rom.nds").unwrap();
        let mut nds = NDS::new(bios7, bios9, fw_path, rom, save_path.clone());

        // Run well past the save-chip boot polling seen in
        // `test_load_state_real_rom_size_and_no_freeze`, so multiple SPI
        // chip-select release/flush cycles have happened.
        for _ in 0..300 {
            nds.emulate_frame();
        }

        // The mmap-backed implementation would fail every operation below
        // with a sharing violation, since the file was mapped for the
        // entire `NDS` lifetime. With no live file handle held between SPI
        // transactions, all of these must succeed while `nds` is still
        // alive and still emulating.
        assert!(save_path.exists(), "expected a .sav file to exist after boot polling");

        let renamed_path = std::env::temp_dir().join("lunaris_test_lock_renamed.sav");
        let _ = std::fs::remove_file(&renamed_path);
        std::fs::rename(&save_path, &renamed_path)
            .expect(".sav file must not be locked: rename failed while NDS is still running");

        std::fs::write(&save_path, vec![0x11u8; 0x8_0000]).expect(
            ".sav file must not be locked: replace-by-write failed while NDS is still running",
        );

        let imported = vec![0x77u8; 0x8_0000];
        nds.import_save(&imported);
        assert_eq!(
            nds.export_save(),
            imported,
            "import_save/export_save should round-trip while the emulator is running"
        );

        // The emulator must keep functioning normally after all of the
        // above -- the point of the fix is that these operations are safe
        // mid-session, not just after shutdown.
        for _ in 0..60 {
            nds.emulate_frame();
        }

        let _ = std::fs::remove_file(&renamed_path);
    }
}
