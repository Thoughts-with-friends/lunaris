//! Save files and savestates: the commands, and the dialogs behind the
//! "File..." entries.

use crate::app::*;

impl MelonEgui {
    /// Ask for a save file to write into the cart's backup memory.
    pub(crate) fn import_savefile(&mut self) {
        self.ask(
            DialogPurpose::ImportSave,
            crate::file::picker::Request::open("Import a save file")
                .filter("save file", &["sav", "dsv", "bin", "dat", "duc", "sa1"])
                // A save carried over from another emulator has whatever name
                // that emulator gave it, and one the dialog will not show is one
                // that cannot be imported at all.
                .any_file()
                .directory(self.dialog_dir("saves")),
        );
    }

    /// Perform the import the dialog asked about.
    ///
    /// Says what it did in enough detail to explain a disappointing result:
    /// which file, how big it was, and how big the cart's own backup memory is.
    /// A save of the wrong size still goes in -- melonDS pads or truncates to
    /// the cart's size -- but a game that then shows no data is explained by
    /// that line and by nothing else.
    pub(crate) fn import_savefile_from(&mut self, path: &Path) {
        if self.emu.is_none() {
            return self.post_error(
                "import failed: no cart is running - open the ROM first, then import its save",
            );
        }
        let data = match std::fs::read(path) {
            Ok(data) if data.is_empty() => {
                return self.post_error(format!("import failed: {} is empty", path.display()));
            }
            Ok(data) => data,
            Err(e) => return self.post_error(format!("cannot read {}: {e}", path.display())),
        };

        // Foreign formats first: what is on disk is not always the raw image
        // the cart wants, and handing a footer to the console is how an import
        // that "worked" leaves a game with no save in it.
        let (raw, note) = raw_save(&data);
        let raw = raw.to_vec();
        let wanted = self.emu.as_mut().map_or(0, |emu| emu.nds.save_memory().len());
        log::info!(
            "importing {}: {} bytes on disk, {} raw, cart holds {wanted}",
            path.display(),
            data.len(),
            raw.len(),
        );
        if let Some(note) = note {
            self.post_warn(note);
        }

        let Some(emu) = self.emu.as_mut() else { return };
        match emu.import_save(&raw) {
            // Importing restarts the console, so nothing from before survives.
            Ok(()) => {
                self.undo_state = None;
                self.frames_run = 0;
                self.post_ok(format!(
                    "imported {} ({}) - console restarted",
                    path.display(),
                    describe_fit(raw.len(), wanted),
                ));
            }
            Err(e) => self.post_error(format!("import failed: {e}")),
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
                crate::file::picker::Request::save("Save state")
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
                self.post_ok(format!("state saved to {} ({mib:.1} MiB)", path.display()));
            }
            Err(e) => self.post_error(format!("save state failed: {e}")),
        }
    }

    /// As [`Self::save_state`]: a slot acts at once, "File..." asks first.
    pub(crate) fn load_state(&mut self, slot: Option<u8>) {
        let Some(emu) = &mut self.emu else { return };
        let Some(slot) = slot else {
            return self.ask(
                DialogPurpose::LoadState,
                crate::file::picker::Request::open("Load state")
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
                self.post_ok(format!("state loaded from {}", path.display()));
            }
            Err(e) => self.post_error(format!("load state failed: {e}")),
        }
    }

    pub(crate) fn undo_state_load(&mut self) {
        let Some(emu) = &mut self.emu else { return };
        let Some(before) = self.undo_state.take() else {
            return;
        };
        match emu.nds.load_state(&before) {
            Ok(()) => self.post_ok("state load undone"),
            Err(e) => self.post_error(format!("undo failed: {e}")),
        }
    }

    // -- the emulation loop -------------------------------------------------
}

/// The trailer DeSmuME appends to a `.dsv`, and the line at the head of it that
/// says what to do about it.
///
/// Everything from that line on is DeSmuME's bookkeeping, not save data: a
/// `.dsv` handed to a cart whole is a save with a hundred-odd bytes of English
/// on the end of it, which is a save no game recognises. This is the "snip" the
/// file itself asks for, in as many words.
const DESMUME_COOKIE: &[u8] = b"|-DESMUME SAVE-|";
const DESMUME_SNIP: &[u8] =
    b"|<--Snip above here to create a raw sav by excluding this DeSmuME savedata footer:";

/// The raw backup image inside `data`, and what had to be done to get it.
///
/// Only DeSmuME's `.dsv` is recognised, because it is the one common foreign
/// format that is *raw data plus a marked trailer*. Anything else -- a `.duc`'s
/// header, a compressed state -- would be a converter rather than a trim, and
/// guessing at one is how a save gets quietly corrupted.
fn raw_save(data: &[u8]) -> (&[u8], Option<&'static str>) {
    if !data.ends_with(DESMUME_COOKIE) {
        return (data, None);
    }
    match find_last(data, DESMUME_SNIP) {
        Some(at) => (&data[..at], Some("DeSmuME .dsv: its footer was trimmed off")),
        // The cookie is there but the line above it is not, so this is a
        // version of the format this does not know. Left alone rather than cut
        // by a guessed length: a save cut in the wrong place is worse than one
        // that is visibly too long.
        None => (data, Some("that looks like a DeSmuME save, but its footer is not one I know")),
    }
}

/// Where `needle` last starts inside `haystack`.
fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).rev().find(|&at| &haystack[at..at + needle.len()] == needle)
}

/// How an imported image compares with the cart's own backup memory, for the
/// message that says the import happened.
fn describe_fit(imported: usize, cart: usize) -> String {
    let kib = |bytes: usize| format!("{:.0} KiB", bytes as f64 / 1024.0);
    if cart == 0 {
        return format!("{}; this cart reports no backup memory", kib(imported));
    }
    match imported.cmp(&cart) {
        std::cmp::Ordering::Equal => kib(imported),
        std::cmp::Ordering::Less => {
            format!("{} into {} - padded; the game may not recognise it", kib(imported), kib(cart))
        }
        std::cmp::Ordering::Greater => {
            format!(
                "{} into {} - truncated; check it is this cart's save",
                kib(imported),
                kib(cart)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DESMUME_COOKIE, DESMUME_SNIP, describe_fit, find_last, raw_save};

    /// A `.dsv` as DeSmuME writes one: the raw image, the snip line, its
    /// fields, then the cookie.
    fn desmume_save(raw: &[u8]) -> Vec<u8> {
        let mut file = raw.to_vec();
        file.extend_from_slice(DESMUME_SNIP);
        file.extend_from_slice(&[0u8; 24]);
        file.extend_from_slice(DESMUME_COOKIE);
        file
    }

    #[test]
    fn a_desmume_save_is_trimmed_back_to_its_raw_image() {
        let raw = vec![0xA5u8; 512];
        let file = desmume_save(&raw);
        let (image, note) = raw_save(&file);
        assert_eq!(image, &raw[..], "the footer is not part of the save");
        assert!(note.is_some(), "a trim nobody asked for has to be said out loud");
    }

    #[test]
    fn an_ordinary_save_is_passed_through_untouched() {
        let raw = vec![0x5Au8; 512];
        let (image, note) = raw_save(&raw);
        assert_eq!(image, &raw[..]);
        assert!(note.is_none());
    }

    /// The cookie without the line above it is a format this does not know, and
    /// a guessed trim would corrupt the save.
    #[test]
    fn an_unknown_desmume_footer_is_left_alone_and_reported() {
        let mut file = vec![0u8; 64];
        file.extend_from_slice(DESMUME_COOKIE);
        let (image, note) = raw_save(&file);
        assert_eq!(image.len(), file.len(), "nothing is cut off on a guess");
        assert!(note.is_some());
    }

    #[test]
    fn the_last_occurrence_is_the_one_found() {
        assert_eq!(find_last(b"ab-ab-ab", b"ab"), Some(6));
        assert_eq!(find_last(b"abc", b"z"), None);
        assert_eq!(find_last(b"a", b"abc"), None, "a needle longer than the hay");
    }

    #[test]
    fn a_mismatched_size_is_described_rather_than_hidden() {
        assert_eq!(describe_fit(524_288, 524_288), "512 KiB");
        assert!(describe_fit(65_536, 524_288).contains("padded"));
        assert!(describe_fit(1_048_576, 524_288).contains("truncated"));
    }
}
