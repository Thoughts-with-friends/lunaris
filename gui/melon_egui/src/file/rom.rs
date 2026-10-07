//! Booting a cart: what `File ▸ Open ROM...` does once a file is chosen.
//!
//! # Flow of [`MelonEgui::load`]
//!
//! 1. Unload the running cart (its save is flushed as it is dropped).
//! 2. Boot the new one with a seat on the shared airwaves (seat 0), so a
//!    second console can link with it later.
//! 3. Read its `.mch` cheats, remember it in the recent list, resume.

use crate::app::*;

impl MelonEgui {
    /// Boot `rom`, replacing whatever was running.
    pub(crate) fn load(&mut self, rom: &Path) {
        crate::file::settings::ensure_instance_layout();
        self.unload_cart();
        self.frames_run = 0;
        self.applied_render = None;
        self.applied_renderer = None;
        // The seat is taken now, at boot: a console's `Host` is fixed when the
        // core is built, so one booted without a seat could never join a link
        // later (this is what local play failing used to look like).
        let booted = Emu::boot_mp(
            rom,
            self.save_dir.as_ref(),
            self.state_dir.as_ref(),
            0,
            self.airwaves.client(0),
        );
        match booted {
            Ok(emu) => {
                self.emu = Some(emu);
                self.reload_cheats(rom);
                if !self.cheats.is_empty() {
                    // Logged: a code file changes what the console does, and a
                    // run that picked one up silently cannot be explained later.
                    let on = self.cheats.iter().filter(|cheat| cheat.enabled).count();
                    log::info!(
                        "{} cheat codes from {}, {on} enabled, engine {}",
                        self.cheats.len(),
                        Self::cheat_path(rom).display(),
                        if self.cheats_enabled { "on" } else { "off" }
                    );
                }
                self.push_recent(rom);
                self.resume_fresh();
                self.post_ok(self.i18n().f(K::RomLoaded, &[&rom.display()]));
            }
            Err(e) => self.post_error(self.i18n().f(K::RomLoadFailed, &[&rom.display(), &e])),
        }
    }

    /// Drop the running console and everything tied to it. Dropping the
    /// [`Emu`] flushes its save.
    pub(crate) fn unload_cart(&mut self) {
        self.emu = None;
        self.drop_link();
        self.textures = None;
        self.undo_state = None;
    }

    /// Read `rom`'s `.mch` as the cheat list (a new list: no selection, and
    /// pushed to the core on the next frame).
    pub(crate) fn reload_cheats(&mut self, rom: &Path) {
        self.cheats = mch::load(&Self::cheat_path(rom)).unwrap_or_default();
        self.select_cheat(None);
        self.applied_cheats = None;
    }

    /// Unpause with a clean pacing window, as after booting a cart.
    pub(crate) fn resume_fresh(&mut self) {
        self.paused = false;
        self.frame_debt = 0.0;
        self.last_tick = Instant::now();
    }
}
