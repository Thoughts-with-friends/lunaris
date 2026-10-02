//! A booted console ([`Emu`]) and the operations every caller shares.
//!
//! # Boot sequence ([`Emu::boot_inner`])
//!
//! 1. Read the ROM file, and the `.sav` beside it (or in the save directory).
//! 2. Build the [`HostBridge`] the core will call back into (save sink, stop
//!    reason, wireless).
//! 3. `Nds::new` → set the RTC → `boot()` (direct boot with FreeBIOS; no BIOS
//!    or firmware files are needed).
//!
//! Anything that talks to the raw core goes through `emu.nds` directly; the
//! methods here exist only where two or more callers would otherwise repeat
//! the same few lines (savestates, input, audio, stop handling).

use super::*;

/// A booted cart, plus the host-side state that outlives any single frame.
pub struct Emu {
    pub nds: Nds,
    /// Where the ROM came from: window titles, and the save/state file names.
    pub rom_path: PathBuf,
    pub info: CartInfo,
    /// The other end of the [`SaveSink`] given to the core.
    pub(crate) saves: Arc<SaveSink>,
    /// Where savestates go; `None` means beside the ROM.
    pub(crate) state_dir: Option<PathBuf>,
    /// Why the core stopped, filled in by the host callback during `run_frame`.
    pub(crate) stop: Arc<Mutex<Option<StopReason>>>,
    /// This console's airwaves seat and instance number, kept so a reboot
    /// (save import) rejoins the same seat.
    pub(crate) seat: Option<(u32, crate::mp::Client)>,
}

impl Emu {
    /// Boot with no wireless and the files beside the ROM (the self test).
    pub fn boot(rom_path: &Path) -> Result<Self, String> {
        Self::boot_with(rom_path, None, None)
    }

    /// Boot with no wireless, the save and state directories overridden.
    pub fn boot_with(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
    ) -> Result<Self, String> {
        Self::boot_inner(rom_path, save_dir, state_dir, 0, None, None)
    }

    /// Boot as console `instance_id` on the shared in-process airwaves.
    ///
    /// `instance_id` also makes the generated firmware's MAC address unique,
    /// as melonDS's own front end does, so two consoles can tell each other
    /// apart.
    pub fn boot_mp(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
        instance_id: u32,
        mp: crate::mp::Client,
    ) -> Result<Self, String> {
        Self::boot_inner(rom_path, save_dir, state_dir, instance_id, Some(mp), None)
    }

    /// Boot with a LAN link as the wireless back end.
    pub fn boot_lan(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
        transport: Box<dyn melonds::Host>,
    ) -> Result<Self, String> {
        Self::boot_inner(rom_path, save_dir, state_dir, 0, None, Some(transport))
    }

    pub(crate) fn boot_inner(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
        instance_id: u32,
        mp: Option<crate::mp::Client>,
        network: Option<Box<dyn melonds::Host>>,
    ) -> Result<Self, String> {
        let rom = std::fs::read(rom_path).map_err(|e| format!("cannot read ROM: {e}"))?;
        let save_path = crate::file::settings::Settings::redirect(save_dir, rom_path, "sav");
        let save = std::fs::read(&save_path).ok();

        let saves = Arc::new(SaveSink { path: save_path, pending: Mutex::new(None) });
        let stop = Arc::new(Mutex::new(None));
        // The core's `Host` owns one airwaves handle; this keeps another for
        // a reboot.
        let seat = mp.clone().map(|mp| (instance_id, mp));
        let host = Box::new(HostBridge {
            saves: Arc::clone(&saves),
            stop: Arc::clone(&stop),
            mp,
            network,
        });

        let mut nds = Nds::new(&rom, save.as_deref(), instance_id, host)
            .map_err(|e| format!("cart rejected: {e}"))?;
        let (y, mo, d, h, mi, s) = if deterministic_rtc() { FIXED_RTC } else { utc_now() };
        nds.set_rtc(y, mo, d, h, mi, s);
        nds.boot();

        Ok(Self {
            nds,
            rom_path: rom_path.to_owned(),
            info: CartInfo::parse(&rom),
            saves,
            state_dir: state_dir.cloned(),
            stop,
            seat,
        })
    }

    /// Run one frame, returning why the console stopped if it did.
    ///
    /// "Stopped" is *asked* (`is_running`), never inferred from a blank frame:
    /// a sleeping console draws nothing and is perfectly healthy.
    pub fn run_frame_checked(&mut self) -> Result<(), String> {
        self.nds.run_frame();
        if self.nds.is_running() {
            return Ok(());
        }
        Err(self.stop_reason().unwrap_or_else(|| "stopped".to_owned()))
    }

    /// Hand the console this frame's buttons and stylus (`None` = lifted).
    pub fn set_input(&mut self, keys: u32, touch: Option<(u16, u16)>) {
        self.nds.set_keys(keys);
        match touch {
            Some((x, y)) => self.nds.touch(x, y),
            None => self.nds.release_screen(),
        }
    }

    /// Take up to `max_pairs` stereo sample pairs of audio out of the core
    /// (interleaved `i16`). Empty when nothing is queued.
    pub fn drain_audio(&mut self, max_pairs: usize) -> Vec<i16> {
        let queued = self.nds.audio_queued().min(max_pairs);
        if queued == 0 {
            return Vec::new();
        }
        let mut buffer = vec![0i16; queued * 2];
        let pairs = self.nds.read_audio(&mut buffer);
        buffer.truncate(pairs * 2);
        buffer
    }

    /// Write out backup memory if the core has changed it since the last call.
    pub fn flush_save(&self) {
        let Some(data) = self.saves.pending.lock().unwrap().take() else {
            return;
        };
        if let Err(e) = std::fs::write(&self.saves.path, &data) {
            log::error!("failed to write {}: {e}", self.saves.path.display());
        }
    }

    /// Why the core stopped, with both CPUs' program counters, taken once.
    pub fn stop_reason(&mut self) -> Option<String> {
        let reason = self.stop.lock().unwrap().take()?;
        Some(format!(
            "{} (ARM9 pc={:08X}, ARM7 pc={:08X})",
            reason.label(),
            self.nds.pc(),
            self.nds.arm7_pc()
        ))
    }

    /// Re-set the real-time clock. It keeps counting in emulated time.
    pub fn set_clock(&mut self, clock: Clock) {
        let Clock { year, month, day, hour, minute, second } = clock;
        self.nds.set_rtc(year, month, day, hour, minute, second);
    }

    /// Savestate path for a numbered slot: melonDS's `<rom>.mlN` convention.
    pub fn state_path(&self, slot: u8) -> PathBuf {
        crate::file::settings::Settings::redirect(
            self.state_dir.as_ref(),
            &self.rom_path,
            &format!("ml{slot}"),
        )
    }

    /// Write a savestate to `path`, returning its size in bytes.
    pub fn save_state_to(&mut self, path: &Path) -> Result<usize, String> {
        let mut buffer = Vec::new();
        self.nds.save_state(&mut buffer).map_err(|e| e.to_string())?;
        std::fs::write(path, &buffer)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok(buffer.len())
    }

    /// Load a savestate from `path`.
    ///
    /// Returns a snapshot of the state *before* the load (for "Undo state
    /// load"), or `None` if that snapshot could not be taken.
    pub fn load_state_from(&mut self, path: &Path) -> Result<Option<Vec<u8>>, String> {
        let mut before = Vec::new();
        let snapshot = self.nds.save_state(&mut before).is_ok();
        let buffer =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        self.nds.load_state(&buffer).map_err(|e| e.to_string())?;
        Ok(snapshot.then_some(before))
    }

    /// Replace the cart's backup memory with `data` and reboot.
    ///
    /// A reboot is the only way: the core reads backup memory once, when it is
    /// constructed. The airwaves seat is kept, so the console stays on the air.
    pub fn import_save(&mut self, data: &[u8]) -> Result<(), String> {
        std::fs::write(&self.saves.path, data)
            .map_err(|e| format!("cannot write {}: {e}", self.saves.path.display()))?;
        // Drop any pending write so the old save cannot land on the new one.
        *self.saves.pending.lock().unwrap() = None;
        let save_dir = self.saves.path.parent().map(Path::to_path_buf);
        let (instance_id, mp) = match self.seat.clone() {
            Some((id, mp)) => (id, Some(mp)),
            None => (0, None),
        };
        *self = Self::boot_inner(
            &self.rom_path,
            save_dir.as_ref(),
            self.state_dir.as_ref(),
            instance_id,
            mp,
            None,
        )?;
        Ok(())
    }
}

impl Drop for Emu {
    /// Unloading a cart is the last chance to persist its save.
    fn drop(&mut self) {
        self.flush_save();
    }
}

/// Whether a framebuffer (`0x00RRGGBB` per pixel) has anything but black.
pub fn has_picture(fb: &[u32]) -> bool {
    fb.iter().any(|&px| px & 0x00FF_FFFF != 0)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::Emu;

    /// Set `MELON_TEST_ROM` to a `.nds` to run these; without it they pass
    /// trivially, since no ROM can be shipped.
    fn test_rom() -> Option<PathBuf> {
        std::env::var_os("MELON_TEST_ROM").map(PathBuf::from).filter(|rom| rom.is_file())
    }

    /// A scratch save directory per test, so tests cannot share a `.sav`.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("melon_egui-save-test").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A save image the cart cannot have written itself.
    fn marked_save(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// The length the cart says its backup memory is.
    fn save_len(rom: &Path) -> usize {
        let dir = scratch("probe");
        let mut emu = Emu::boot_with(rom, Some(&dir), None).unwrap();
        emu.nds.save_memory().len()
    }

    /// A `.sav` where the cart's save belongs must reach the cart.
    #[test]
    fn a_save_file_beside_the_cart_is_in_the_cart() {
        let Some(rom) = test_rom() else { return };
        let len = save_len(&rom);
        assert!(len > 0, "the cart reports no backup memory at all");

        let dir = scratch("boot");
        let save = marked_save(len);
        let path = crate::file::settings::Settings::redirect(Some(&dir), &rom, "sav");
        std::fs::write(&path, &save).unwrap();

        let mut emu = Emu::boot_with(&rom, Some(&dir), None).unwrap();
        assert_eq!(emu.nds.save_memory(), save, "the file did not reach the cart");
    }

    /// `File ▸ Import savefile`: the bytes reach a running cart and the disk.
    #[test]
    fn an_imported_save_reaches_the_running_cart() {
        let Some(rom) = test_rom() else { return };
        let len = save_len(&rom);

        let dir = scratch("import");
        let mut emu = Emu::boot_with(&rom, Some(&dir), None).unwrap();
        let save = marked_save(len);
        emu.import_save(&save).unwrap();

        assert_eq!(emu.nds.save_memory(), save, "the import did not reach the cart");
        let path = crate::file::settings::Settings::redirect(Some(&dir), &rom, "sav");
        assert_eq!(std::fs::read(&path).unwrap(), save, "the import was not written out");
    }

    /// A save of the wrong length is still imported (melonDS pads/truncates).
    #[test]
    fn a_save_of_the_wrong_length_is_still_imported() {
        let Some(rom) = test_rom() else { return };
        let len = save_len(&rom);
        if len < 2 {
            return;
        }

        let dir = scratch("short");
        let mut emu = Emu::boot_with(&rom, Some(&dir), None).unwrap();
        let short = marked_save(len / 2);
        emu.import_save(&short).unwrap();

        let in_cart = emu.nds.save_memory();
        assert_eq!(in_cart.len(), len, "the cart keeps its own size");
        assert_eq!(&in_cart[..short.len()], &short[..], "the file's bytes are not in the cart");
    }
}
