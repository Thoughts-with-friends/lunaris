//! Save files and savestates: the commands, and the dialogs behind the
//! "File..." entries.

use super::*;

impl MelonEgui {
    /// Ask for a save file to write into the cart's backup memory.
    pub(crate) fn import_savefile(&mut self) {
        self.ask(
            DialogPurpose::ImportSave,
            crate::fs::Request::open("Import a save file")
                .filter("save file", &["sav", "dsv", "bin"])
                .directory(self.dialog_dir("saves")),
        );
    }

    /// Perform the import the dialog asked about.
    pub(crate) fn import_savefile_from(&mut self, path: &Path) {
        let outcome = std::fs::read(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))
            .and_then(|data| {
                self.emu
                    .as_mut()
                    .ok_or_else(|| "no cart loaded".to_owned())
                    .and_then(|emu| emu.import_save(&data))
            });
        match outcome {
            // Importing restarts the console, so nothing from before survives.
            Ok(()) => {
                self.undo_state = None;
                self.frames_run = 0;
                self.post(format!("imported {} — console restarted", path.display()));
            }
            Err(e) => self.post(format!("import failed: {e}")),
        }
    }

    /// A numbered slot writes straight away; "File..." asks first and lands in
    /// [`Self::write_state_to`] once the dialog is answered.
    pub(crate) fn save_state(&mut self, slot: Option<u8>) {
        let Some(emu) = &mut self.emu else { return };
        let Some(slot) = slot else {
            // Pre-filled with the cart's own name, so "File..." does not open
            // on an empty name box beside eight slots that are named for you.
            let suggestion = emu
                .rom_path
                .file_stem()
                .map_or_else(|| "state".to_owned(), |stem| stem.to_string_lossy().into_owned());
            let directory = self.dialog_dir("states");
            return self.ask(
                DialogPurpose::SaveState,
                crate::fs::Request::save("Save state")
                    .filter("savestate", &["ml1"])
                    .file_name(format!("{suggestion}.ml1"))
                    .directory(directory),
            );
        };
        let path = emu.state_path(slot);
        self.write_state_to(&path);
    }

    pub(crate) fn write_state_to(&mut self, path: &Path) {
        let Some(emu) = &mut self.emu else { return };
        let mut buf = Vec::new();
        let outcome = emu.nds.save_state(&mut buf).map_err(|e| e.to_string()).and_then(|()| {
            std::fs::write(path, &buf).map_err(|e| format!("cannot write {}: {e}", path.display()))
        });
        match outcome {
            Ok(()) => {
                let mib = buf.len() as f64 / (1024.0 * 1024.0);
                self.post(format!("state saved to {} ({mib:.1} MiB)", path.display()));
            }
            Err(e) => self.post(format!("save state failed: {e}")),
        }
    }

    /// As [`Self::save_state`]: a slot acts at once, "File..." asks first.
    pub(crate) fn load_state(&mut self, slot: Option<u8>) {
        let Some(emu) = &mut self.emu else { return };
        let Some(slot) = slot else {
            return self.ask(
                DialogPurpose::LoadState,
                crate::fs::Request::open("Load state")
                    .filter("savestate", &["ml1"])
                    .directory(self.dialog_dir("states")),
            );
        };
        let path = emu.state_path(slot);
        self.read_state_from(&path);
    }

    pub(crate) fn read_state_from(&mut self, path: &Path) {
        let Some(emu) = &mut self.emu else { return };

        // Snapshot first: a load with nothing to go back to is a load that
        // cannot be undone, and melonDS offers exactly that undo.
        let mut before = Vec::new();
        let snapshot = emu.nds.save_state(&mut before).is_ok();

        let outcome = std::fs::read(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))
            .and_then(|buf| emu.nds.load_state(&buf).map_err(|e| e.to_string()));
        match outcome {
            Ok(()) => {
                self.undo_state = snapshot.then_some(before);
                self.post(format!("state loaded from {}", path.display()));
            }
            Err(e) => self.post(format!("load state failed: {e}")),
        }
    }

    pub(crate) fn undo_state_load(&mut self) {
        let Some(emu) = &mut self.emu else { return };
        let Some(before) = self.undo_state.take() else {
            return;
        };
        match emu.nds.load_state(&before) {
            Ok(()) => self.post("state load undone"),
            Err(e) => self.post(format!("undo failed: {e}")),
        }
    }

    // -- the emulation loop -------------------------------------------------
}
