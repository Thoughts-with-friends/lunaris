//! The cart's Action Replay codes: reading them, editing them, and handing
//! them to both consoles.

use crate::app::*;

impl MelonEgui {
    /// The ROM info pane's rows, or `None` with no cart loaded.
    /// Where a cart's codes live in instance1's dedicated cheat directory.
    /// The file keeps melonDS's `.mch` format so both front ends can use it.
    pub fn cheat_path(rom: &Path) -> PathBuf {
        crate::file::settings::instance_data_dir(1, "cheats").join(rom.file_stem().map_or_else(
            || PathBuf::from("cheats.mch"),
            |name| PathBuf::from(format!("{}.mch", name.to_string_lossy())),
        ))
    }

    /// The path the running cart's codes are read from and written to.
    pub fn cheat_file(&self) -> Option<PathBuf> {
        self.emu.as_ref().map(|emu| Self::cheat_path(&emu.rom_path))
    }

    /// Show `index` in the editor, loading its three boxes from the code it
    /// names. `None` clears the editor.
    ///
    /// The one way the selection is ever set, so that "which row is selected"
    /// and "what the editor holds" cannot drift apart. An index past the end —
    /// which is what a delete or a freshly read file leaves behind — clears
    /// rather than panics.
    pub fn select_cheat(&mut self, index: Option<usize>) {
        match index.and_then(|i| self.cheats.get(i).map(|cheat| (i, cheat))) {
            Some((i, cheat)) => {
                self.cheat_editor = CheatEditor {
                    name: cheat.name.clone(),
                    notes: cheat.description.clone(),
                    code: cheat.text(),
                };
                self.cheat_selected = Some(i);
            }
            None => {
                self.cheat_editor = CheatEditor::default();
                self.cheat_selected = None;
            }
        }
    }

    /// Add an empty code and select it, which is what the editor then fills in.
    ///
    /// Off by default: a code with no words in it would do nothing, and one
    /// that is enabled before it has been written is a code the user did not
    /// ask for.
    pub fn add_cheat(&mut self) {
        self.cheats.push(Cheat {
            name: "New cheat".to_owned(),
            enabled: false,
            ..Cheat::default()
        });
        self.select_cheat(Some(self.cheats.len() - 1));
    }

    /// Write the editor back into the selected code, and the whole list to
    /// disk.
    ///
    /// Committing on a button rather than on every keystroke is deliberate:
    /// `apply_cheats` compares the whole list against what the core was last
    /// given, so live editing would push every code into the console once per
    /// character typed.
    pub fn commit_cheat_editor(&mut self) {
        let Some(index) = self.cheat_selected else {
            self.post_error("no code selected");
            return;
        };
        let editor = self.cheat_editor.clone();
        let code = match mch::parse_code(&editor.code) {
            Ok(code) => code,
            Err(token) => {
                self.post_error(format!("not a 32-bit hex word: {token}"));
                return;
            }
        };
        let odd = !code.len().is_multiple_of(2);
        let empty = code.is_empty();
        let Some(cheat) = self.cheats.get_mut(index) else {
            self.post_error("that code is no longer in the list");
            return;
        };
        cheat.name = if editor.name.trim().is_empty() { "Unnamed".to_owned() } else { editor.name };
        cheat.description = editor.notes;
        cheat.code = code;
        self.save_cheats();
        if empty {
            self.post_warn("saved, but that code has no words in it");
        } else if odd {
            self.post_warn("saved, but that code has an odd number of words");
        }
    }

    /// Move the code at `from` to sit at `to`, and write the new order out.
    ///
    /// Saved on the drop rather than on a later Save press: the order is a
    /// property of the list itself, not of the code the editor happens to be
    /// showing, and there is nowhere else it could be committed from -- Save is
    /// disabled unless a code is selected.
    ///
    /// The selection follows the *code*, not the row number, and the editor is
    /// deliberately left alone: nothing about the code changed, so reordering
    /// must not discard whatever is half-typed in it.
    pub fn move_cheat(&mut self, from: usize, to: usize) {
        if from == to || from >= self.cheats.len() || to >= self.cheats.len() {
            return;
        }
        let cheat = self.cheats.remove(from);
        self.cheats.insert(to, cheat);
        self.cheat_selected = self.cheat_selected.map(|selected| shift(selected, from, to));
        self.save_cheats();
    }

    /// Remove the selected code, selecting whatever takes its place.
    pub fn delete_selected_cheat(&mut self) {
        let Some(index) = self.cheat_selected else { return };
        if index >= self.cheats.len() {
            self.select_cheat(None);
            return;
        }
        let removed = self.cheats.remove(index);
        // The row that slid up into the gap, or the new last row when the one
        // removed was the last -- either way the selection stays on screen.
        let next = index.min(self.cheats.len().saturating_sub(1));
        self.select_cheat((!self.cheats.is_empty()).then_some(next));
        // Written straight away rather than left for Save: Save needs a
        // selected code, and deleting the last one leaves nothing to select --
        // so a deletion that waited for it could never be persisted at all.
        self.save_cheats();
        self.post_ok(format!("removed {}", removed.name));
    }

    /// Write the current list back to the cart's `.mch`.
    pub fn save_cheats(&mut self) {
        let Some(path) = self.cheat_file() else { return };
        match mch::save(&path, &self.cheats) {
            Ok(()) => self.post_ok(format!("cheats written to {}", path.display())),
            Err(e) => self.post_error(e),
        }
    }

    /// Read a `.mch` the user picked, replacing the list.
    pub fn import_cheats(&mut self, path: &Path) {
        match mch::load(path) {
            Ok(list) => {
                let count = list.len();
                self.cheats = list;
                // The list the selection indexed into is gone.
                self.select_cheat(None);
                self.post_ok(format!("{count} codes read from {}", path.display()));
            }
            Err(e) => self.post_error(e),
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
            crate::file::picker::Request::open("Open melonDS cheats")
                .filter("melonDS cheats", &["mch"])
                .directory(self.dialog_dir("cheats")),
        );
    }
}

/// Where row `index` ends up once the code at `from` is moved to `to`.
///
/// Every row between the two shifts by one towards the gap the move left
/// behind; the moved row itself lands on `to`. Split out as a free function
/// because it is pure arithmetic that is easy to get subtly wrong in either
/// direction, and because that makes it testable without a console.
const fn shift(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

#[cfg(test)]
mod tests {
    use super::shift;

    #[test]
    fn a_moved_row_takes_its_selection_with_it() {
        // 0 1 2 3 4, moving 1 down to 3: 0 2 3 1 4.
        assert_eq!(shift(1, 1, 3), 3, "the moved row lands where it was dropped");
        assert_eq!(shift(2, 1, 3), 1, "the rows it passed slid up");
        assert_eq!(shift(3, 1, 3), 2);
        assert_eq!(shift(0, 1, 3), 0, "rows outside the range do not move");
        assert_eq!(shift(4, 1, 3), 4);
    }

    #[test]
    fn moving_a_row_up_shifts_the_other_way() {
        // 0 1 2 3 4, moving 3 up to 1: 0 3 1 2 4.
        assert_eq!(shift(3, 3, 1), 1);
        assert_eq!(shift(1, 3, 1), 2);
        assert_eq!(shift(2, 3, 1), 3);
        assert_eq!(shift(0, 3, 1), 0);
        assert_eq!(shift(4, 3, 1), 4);
    }
}
