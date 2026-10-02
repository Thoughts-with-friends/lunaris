//! The second console: opening it, closing it, and giving it orders.
//!
//! # Launch ([`MelonEgui::launch_instance`])
//!
//! 1. Give it its own save directory, `instance2/saves`, seeded with a copy of
//!    the first console's save (two consoles are two carts; one shared `.sav`
//!    would be overwritten by whichever wrote last).
//! 2. Start it at the first console's frame count — the wifi clock's epoch —
//!    so the two read each other's traffic as current.
//! 3. Spawn it on its own thread ([`crate::guest::Guest::spawn`]), on seat 1
//!    of the shared airwaves; in Remote Desktop mode its picture and sound go
//!    to the remote player.

use super::*;

impl MelonEgui {
    /// Hand a command to the second console, if there is one.
    pub fn command_guest(&mut self, command: crate::guest::Command) {
        match &self.guest {
            Some(guest) => guest.send(command),
            None => self.post_warn("no second console is running"),
        }
    }

    /// Close the second console, if one is open.
    pub(crate) fn close_guest(&mut self) {
        self.guest = None;
        self.guest_textures = None;
    }

    /// The second console's save directory, seeded with the first console's
    /// save. `None` (share the first one's) if the directory cannot be made.
    pub(crate) fn guest_save_dir(&mut self, rom: &Path) -> Option<PathBuf> {
        let host_save = Settings::redirect(self.save_dir.as_ref(), rom, "sav");
        let dir = crate::file::settings::instance_data_dir(2, "saves");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.post_warn(format!("cannot make {}: {e}; sharing the save", dir.display()));
            return None;
        }
        let guest_save = Settings::redirect(Some(&dir), rom, "sav");
        if !guest_save.exists()
            && host_save.exists()
            && let Err(e) = std::fs::copy(&host_save, &guest_save)
        {
            self.post_error(format!("cannot seed {}: {e}", guest_save.display()));
        }
        Some(dir)
    }

    /// Open a second console on the same cart and airwaves — or close it if
    /// one is open. (melonDS launches a second process for this.)
    pub(crate) fn launch_instance(&mut self) {
        if self.guest.is_some() {
            self.close_guest();
            self.post("second instance closed");
            return;
        }
        let Some(rom) = self.emu.as_ref().map(|emu| emu.rom_path.clone()) else {
            self.post_warn("load a cart first");
            return;
        };
        let save_dir = self.guest_save_dir(&rom);
        // melonDS starts a wifi clock at `frames * 16716`, so a console booted
        // mid-session must start from the first console's frame count.
        let start_frame = self.emu.as_mut().map_or(0, |host| host.nds.frame_count());
        let stream = self.remote_host.clone();
        let streamed = stream.is_some();
        self.guest = Some(crate::guest::Guest::spawn(
            &rom,
            save_dir,
            Some(crate::file::settings::instance_data_dir(2, "states")),
            Some(crate::file::settings::instance_data_dir(2, "cheats")),
            1,
            self.airwaves.client(1),
            start_frame,
            stream,
        ));
        self.post(if streamed {
            "second instance launched — its picture and sound go to the remote player"
        } else {
            "second instance launched - both consoles share the airwaves"
        });
    }

    /// Show the first console's instance directory in the file manager.
    pub(crate) fn open_directory(&mut self) {
        self.open_instance_directory(1);
    }

    /// Show `instanceN/` (saves, states, cheats) in the file manager.
    pub(crate) fn open_instance_directory(&mut self, instance: u32) {
        self.reveal(&crate::file::settings::instance_dir(instance));
    }
}
