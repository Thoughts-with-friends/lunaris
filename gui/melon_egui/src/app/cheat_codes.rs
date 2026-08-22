//! The cart's Action Replay codes: reading them, editing them, and handing
//! them to both consoles.

use super::*;

impl MelonEgui {
    /// The ROM info pane's rows, or `None` with no cart loaded.
    /// Where a cart's codes live in instance1's dedicated cheat directory.
    /// The file keeps melonDS's `.mch` format so both front ends can use it.
    pub fn cheat_path(rom: &Path) -> PathBuf {
        crate::config::instance_data_dir(1, "cheats").join(rom.file_stem().map_or_else(
            || PathBuf::from("cheats.mch"),
            |name| PathBuf::from(format!("{}.mch", name.to_string_lossy())),
        ))
    }

    /// The path the running cart's codes are read from and written to.
    pub fn cheat_file(&self) -> Option<PathBuf> {
        self.emu.as_ref().map(|emu| Self::cheat_path(&emu.rom_path))
    }

    /// Turn the dialog's two boxes into a code, reporting a bad paste rather
    /// than adding something the engine would read as garbage.
    pub fn add_cheat_from_draft(&mut self) {
        let (name, text) = self.cheat_draft.clone();
        match cheats::parse_code(&text) {
            Ok(code) if code.is_empty() => self.post("no code words in that text"),
            Ok(code) => {
                let odd = !code.len().is_multiple_of(2);
                self.cheats.push(Cheat {
                    name: if name.trim().is_empty() { "Unnamed".to_owned() } else { name },
                    code,
                    enabled: true,
                    ..Cheat::default()
                });
                self.cheat_draft = (String::new(), String::new());
                if odd {
                    self.post("added, but that code has an odd number of words");
                }
            }
            Err(token) => self.post(format!("not a 32-bit hex word: {token}")),
        }
    }

    /// Write the current list back to the cart's `.mch`.
    pub fn save_cheats(&mut self) {
        let Some(path) = self.cheat_file() else { return };
        match cheats::save(&path, &self.cheats) {
            Ok(()) => self.post(format!("cheats written to {}", path.display())),
            Err(e) => self.post(e),
        }
    }

    /// Read a `.mch` the user picked, replacing the list.
    pub fn import_cheats(&mut self, path: &Path) {
        match cheats::load(path) {
            Ok(list) => {
                let count = list.len();
                self.cheats = list;
                self.post(format!("{count} codes read from {}", path.display()));
            }
            Err(e) => self.post(e),
        }
    }

    /// Hand the core the codes it should be running.
    ///
    /// Only on a change: the list is copied into the console, and it is pushed
    /// from the same place every repaint so that a code toggled in the dialog
    /// takes effect on the next frame.
    pub(crate) fn apply_cheats(&mut self) {
        let wanted = (self.cheats_enabled, self.cheats.clone());
        if self.applied_cheats.as_ref() == Some(&wanted) || self.emu.is_none() {
            return;
        }
        let Some(emu) = &mut self.emu else { return };
        // Cheats off is an empty list rather than a flag: melonDS runs whatever
        // is in the console's list, so this is the only way to stop it.
        let installed: Vec<melonds::Cheat> = if self.cheats_enabled {
            self.cheats.iter().map(Cheat::to_core).collect()
        } else {
            Vec::new()
        };
        emu.nds.set_cheats(&installed);
        // The second console is a second cart, and a cheat that is on for one
        // player and off for the other desynchronises a linked game outright.
        // Its own `.mch` was read when it booted; this is the master switch and
        // any code edited since, which have to reach both.
        if let Some(guest) = &self.guest {
            guest.send(crate::guest::Command::SetCheats(installed.clone()));
        }
        self.applied_cheats = Some(wanted);
    }

    /// Ask for a `.mch` cheat file to merge into the current cart's list.
    pub fn ask_for_cheat_file(&mut self) {
        self.ask(
            DialogPurpose::ImportCheats,
            crate::fs::Request::open("Open melonDS cheats")
                .filter("melonDS cheats", &["mch"])
                .directory(self.dialog_dir("cheats")),
        );
    }
}
