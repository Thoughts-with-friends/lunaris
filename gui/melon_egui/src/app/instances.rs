//! The second console: opening it, closing it, and giving it orders.

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

    /// Where the second console's backup memory goes: `instance2/` under
    /// console 0's save directory, seeded with a copy of console 0's file so
    /// the two start from the same progress and then diverge, as two carts do.
    ///
    /// `None` if the directory cannot be made, which falls back to sharing —
    /// worse, but better than refusing to launch.
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

    /// Open a second console on the same cart and the same airwaves, which is
    /// what makes local wireless play testable in one window.
    ///
    /// melonDS launches a whole second process for this; here it is a second
    /// [`Emu`] driven from the same repaint, which keeps the two wifi clocks in
    /// step without any cross-process synchronisation.
    pub(crate) fn launch_instance(&mut self) {
        if self.guest.is_some() {
            self.guest = None;
            self.guest_textures = None;
            self.post("second instance closed");
            return;
        }
        let Some(rom) = self.emu.as_ref().map(|emu| emu.rom_path.clone()) else {
            self.post_warn("load a cart first");
            return;
        };
        // Console 0 already holds seat 0 (see `load`), so only the second
        // console is booted here.
        //
        // It gets a save directory of its own, seeded from console 0's file:
        // two consoles are two carts, and pointing both at one `.sav` means
        // whichever writes last wins -- with a real save on the line.
        let save_dir = self.guest_save_dir(&rom);
        // Put the newcomer on console 0's wireless timebase. melonDS starts a
        // console's wifi clock at `frames * 16716` when wifi powers on, so a
        // console booted mid-session would stamp its frames however far behind
        // it started -- minutes, here -- and the two would read each other's
        // traffic as ancient.
        let start_frame = self.emu.as_mut().map_or(0, |host| host.nds.frame_count());
        // In Remote Desktop mode this console is the remote player's: its
        // picture and sound go out over the session, and its controls come back
        // from it. See `crate::remote`.
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

    /// Show this front end's directory in the system file manager.
    pub(crate) fn open_directory(&mut self) {
        self.open_instance_directory(1);
    }

    /// Show one instance's directory, so the second console's window opens its
    /// own `saves`/`states`/`cheats` rather than the first console's.
    pub(crate) fn open_instance_directory(&mut self, instance: u32) {
        let dir = crate::file::settings::instance_dir(instance);
        self.reveal(&dir);
    }
}
