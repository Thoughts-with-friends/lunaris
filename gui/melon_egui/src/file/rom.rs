//! Booting a cart.
//!
//! The whole of what `File ▸ Open ROM...` does once the picker has answered:
//! drop the old console, boot the new one, pick up the cheats that sit beside
//! it, and remember it in the recent list.

use crate::app::*;

impl MelonEgui {
    /// Boot `rom`, replacing whatever was running.
    pub(crate) fn load(&mut self, rom: &Path) {
        crate::file::settings::ensure_instance_layout();
        // Dropped first so the outgoing cart's save is flushed before the
        // incoming one can be handed the same file.
        self.emu = None;
        self.drop_link();
        self.undo_state = None;
        self.textures = None;
        self.frames_run = 0;
        self.applied_render = None;
        self.applied_renderer = None;
        // Console 0 takes its seat on the airwaves here, at boot, rather than
        // when a second instance is launched: a console's `Host` is fixed when
        // the core is constructed, so one booted without a seat can never join
        // afterwards — its frames vanish and its peer hears silence, which is
        // what local play failing looked like. A seat costs nothing until the
        // cart calls `MP_Begin`.
        match Emu::boot_mp(
            rom,
            self.save_dir.as_ref(),
            self.state_dir.as_ref(),
            0,
            self.airwaves.client(0),
        ) {
            Ok(emu) => {
                self.emu = Some(emu);
                self.cheats = mch::load(&Self::cheat_path(rom)).unwrap_or_default();
                self.applied_cheats = None;
                if !self.cheats.is_empty() {
                    // Worth saying out loud: a code file found beside the ROM
                    // changes what the console does, and a run that picked one
                    // up silently is a run nobody can explain afterwards.
                    let on = self.cheats.iter().filter(|cheat| cheat.enabled).count();
                    log::info!(
                        "{} cheat codes from {}, {on} enabled, engine {}",
                        self.cheats.len(),
                        Self::cheat_path(rom).display(),
                        if self.cheats_enabled { "on" } else { "off" }
                    );
                }
                self.push_recent(rom);
                self.paused = false;
                self.frame_debt = 0.0;
                self.last_tick = Instant::now();
                self.post_ok(format!("loaded {}", rom.display()));
            }
            Err(e) => self.post_error(format!("failed to load {}: {e}", rom.display())),
        }
    }
}
