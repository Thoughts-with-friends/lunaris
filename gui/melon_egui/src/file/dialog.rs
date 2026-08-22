//! Opening a system file dialog, and acting on its answer a few repaints
//! later. See [`crate::file::picker`] for why it cannot simply block.

use crate::app::*;

impl MelonEgui {
    /// Open a system file dialog, off the UI thread.
    ///
    /// Refuses while one is already open rather than stacking two unparented
    /// dialogs the user cannot tell apart. See [`crate::file::picker`] for why this is not
    /// a blocking call.
    pub(crate) fn ask(&mut self, purpose: DialogPurpose, request: crate::file::picker::Request) {
        if self.dialog.is_some() {
            self.post_warn("a file dialog is already open");
            return;
        }
        match crate::file::picker::Pending::spawn(purpose, request) {
            Ok(pending) => self.dialog = Some(pending),
            // Reported rather than swallowed: from the user's side a dialog
            // that never appears is a menu entry that was ignored.
            Err(error) => self.post_error(error),
        }
    }

    /// Act on a dialog the user has finished with.
    ///
    /// Called once per repaint from [`Self::advance`], which is what keeps the
    /// window drawing and the console running while a dialog is on screen.
    pub(crate) fn poll_dialog(&mut self) {
        let Some((purpose, path)) = crate::file::picker::Pending::take_answer(&mut self.dialog)
        else {
            return;
        };
        // Cancelled. Not worth an OSD message: the user knows they cancelled.
        let Some(path) = path else { return };
        match purpose {
            DialogPurpose::OpenRom => self.load(&path),
            DialogPurpose::ImportSave => self.import_savefile_from(&path),
            DialogPurpose::SaveState => self.write_state_to(&path),
            DialogPurpose::LoadState => self.read_state_from(&path),
            DialogPurpose::ImportCheats => self.import_cheats(&path),
            DialogPurpose::GuestImportSave => match std::fs::read(&path) {
                Ok(data) => self.command_guest(crate::guest::Command::ImportSave(data)),
                Err(error) => self.post_error(format!("cannot read {}: {error}", path.display())),
            },
            DialogPurpose::GuestSaveState => {
                self.command_guest(crate::guest::Command::SaveState(None, Some(path)));
            }
            DialogPurpose::GuestLoadState => {
                self.command_guest(crate::guest::Command::LoadState(None, Some(path)));
            }
            DialogPurpose::Directory(setting) => {
                setting.set(self, path);
                self.persist();
            }
        }
    }

    /// Where a dialog for `extension` should open: this instance's own
    /// directory when there is one, so a savestate dialog lands in `states`
    /// rather than wherever the system last was.
    pub(crate) fn dialog_dir(&self, kind: &str) -> Option<PathBuf> {
        match kind {
            "saves" => self.save_dir.clone(),
            "states" => self.state_dir.clone(),
            _ => Some(crate::file::settings::instance_data_dir(1, kind)),
        }
    }

    /// Ask for a directory for one of the Path settings.
    pub fn ask_for_directory(&mut self, setting: crate::ui::panes::PathSetting) {
        self.ask(
            DialogPurpose::Directory(setting),
            crate::file::picker::Request::folder("Choose a directory")
                .directory(Some(crate::file::settings::instances_dir())),
        );
    }
}
