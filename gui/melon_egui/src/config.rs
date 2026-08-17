//! What the front end remembers between runs, and where it keeps it.
//!
//! melonDS stores its configuration in a directory of its own, which its File
//! menu can open; this is the equivalent. Deliberately small: only the things a
//! user would be annoyed to set twice.

use std::path::{Path, PathBuf};

use crate::view::ViewOptions;

/// How many recent ROMs are remembered, matching melonDS's list length.
pub const RECENT_LIMIT: usize = 10;

/// The persisted settings.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Most recently opened first.
    pub recents: Vec<PathBuf>,
    pub view: ViewOptions,
    pub limit_framerate: bool,
    pub audio_sync: bool,
    /// Where savestates go. Empty means "beside the ROM", melonDS's default.
    pub state_dir: Option<PathBuf>,
    /// Where `.sav` files go. Empty means "beside the ROM".
    pub save_dir: Option<PathBuf>,
    /// egui's UI scale. 0 means "follow the system".
    pub ui_scale: f32,
    pub dark_theme: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recents: Vec::new(),
            view: ViewOptions::default(),
            limit_framerate: true,
            audio_sync: false,
            state_dir: None,
            save_dir: None,
            ui_scale: 0.0,
            // The core's picture is pixel art, and a pale theme washes it out.
            dark_theme: true,
        }
    }
}

impl Settings {
    /// Read the settings file, falling back to defaults on anything unreadable
    /// or unparseable — a corrupt file should not stop the emulator starting.
    pub fn load() -> Self {
        let path = settings_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str(&text) {
            Ok(settings) => settings,
            Err(e) => {
                eprintln!("melon_egui: ignoring unreadable {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// Write the settings out, reporting but not propagating failure: losing the
    /// recent list matters less than interrupting whatever the user was doing.
    pub fn save(&self) {
        let dir = config_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("melon_egui: cannot create {}: {e}", dir.display());
            return;
        }
        let path = settings_path();
        let json = match serde_json::to_string_pretty(self) {
            Ok(json) => json,
            Err(e) => {
                eprintln!("melon_egui: cannot serialize settings: {e}");
                return;
            }
        };
        if let Err(e) = std::fs::write(&path, json) {
            eprintln!("melon_egui: cannot write {}: {e}", path.display());
        }
    }

    /// Record `rom` as the newest entry, moving it up if it was already there and
    /// dropping the oldest past [`RECENT_LIMIT`].
    pub fn push_recent(&mut self, rom: &Path) {
        self.recents.retain(|existing| existing != rom);
        self.recents.insert(0, rom.to_owned());
        self.recents.truncate(RECENT_LIMIT);
    }

    /// Where a file for `rom` belongs, given an override directory: inside it if
    /// one is set, otherwise beside the ROM.
    pub fn redirect(dir: Option<&PathBuf>, rom: &Path, extension: &str) -> PathBuf {
        let with_ext = rom.with_extension(extension);
        match dir {
            Some(dir) => match with_ext.file_name() {
                Some(name) => dir.join(name),
                None => with_ext,
            },
            None => with_ext,
        }
    }
}

/// The directory this front end keeps its files in.
///
/// Beside the executable, so that a copied-out build stays self-contained —
/// which is the point of it being a single file. Falls back to the working
/// directory if the executable's path is not knowable.
pub fn config_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("melon_egui")
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{RECENT_LIMIT, Settings};

    #[test]
    fn recents_put_the_newest_first_without_duplicating() {
        let mut settings = Settings::default();
        settings.push_recent(Path::new("a.nds"));
        settings.push_recent(Path::new("b.nds"));
        settings.push_recent(Path::new("a.nds"));
        assert_eq!(settings.recents, [PathBuf::from("a.nds"), PathBuf::from("b.nds")]);
    }

    #[test]
    fn recents_are_capped() {
        let mut settings = Settings::default();
        for i in 0..RECENT_LIMIT + 5 {
            settings.push_recent(Path::new(&format!("rom{i}.nds")));
        }
        assert_eq!(settings.recents.len(), RECENT_LIMIT);
        // The newest survives and the oldest is gone.
        assert_eq!(settings.recents[0], PathBuf::from("rom14.nds"));
        assert!(!settings.recents.contains(&PathBuf::from("rom0.nds")));
    }

    #[test]
    fn redirect_keeps_the_file_name_and_falls_back_beside_the_rom() {
        let rom = Path::new("D:/roms/game.nds");
        assert_eq!(Settings::redirect(None, rom, "sav"), PathBuf::from("D:/roms/game.sav"));
        assert_eq!(
            Settings::redirect(Some(&PathBuf::from("D:/saves")), rom, "sav"),
            PathBuf::from("D:/saves/game.sav"),
        );
    }

    /// A settings file from a different version must not stop startup.
    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let settings: Settings =
            serde_json::from_str(r#"{"limit_framerate": false, "nonsense": 1}"#).unwrap();
        assert!(!settings.limit_framerate);
        // Everything absent keeps its default.
        assert!(settings.dark_theme);
        assert!(settings.recents.is_empty());
    }
}
