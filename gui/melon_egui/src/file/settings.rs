//! What the front end remembers between runs, and where it keeps it.
//!
//! melonDS stores its configuration in a directory of its own, which its File
//! menu can open; this is the equivalent. Deliberately small: only the things a
//! user would be annoyed to set twice.

use std::path::{Path, PathBuf};

use crate::{
    ui::{panes::Pane, view::ViewOptions},
    video::VideoOptions,
};

/// How many recent ROMs are remembered, matching melonDS's list length.
pub const RECENT_LIMIT: usize = 10;

/// The instance directories used by melon_egui.
pub const INSTANCE_COUNT: u32 = 2;

/// The persisted settings.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Most recently opened first.
    pub recents: Vec<PathBuf>,
    pub view: ViewOptions,
    pub video: VideoOptions,
    /// Dialogs that were open when the window closed.
    pub open_panes: Vec<Pane>,
    pub limit_framerate: bool,
    pub audio_sync: bool,
    /// How fast the console runs relative to real time. See [`crate::speed`].
    pub speed: f32,
    /// melonDS's "Enable cheats", which is a preference rather than a per-cart
    /// thing: the codes themselves live in the cart's `.mch`.
    pub cheats_enabled: bool,
    /// Output volume, 0.0 to 1.0.
    pub volume: f32,
    /// Where savestates go. Empty means "beside the ROM", melonDS's default.
    pub state_dir: Option<PathBuf>,
    /// Where `.sav` files go. Empty means "beside the ROM".
    pub save_dir: Option<PathBuf>,
    /// egui's UI scale. 0 means "follow the system".
    pub ui_scale: f32,
    pub dark_theme: bool,
    /// Which translation the UI is drawn in.
    pub language: crate::i18n::Language,
    /// The address the guest join box is pre-filled with — the last one that
    /// was typed into it. Persisted because on a VPN it is a number nobody
    /// remembers and it does not change between sessions.
    pub lan_host_address: String,
    /// The address the host binds. Persisted for the same reason, though it
    /// changes less often.
    pub lan_bind_address: String,
    /// How the LAN transport behaves on a slow link. See [`crate::lan::Tuning`].
    pub lan: crate::lan::Tuning,
    /// How Remote Desktop mode behaves. See [`crate::remote::Tuning`].
    pub remote: crate::remote::Tuning,
    /// Which key and which pad button each DS button answers to. Defaults to
    /// melonDS's own map; see [`crate::bindings`].
    pub bindings: crate::bindings::Bindings,

    pub window: crate::app::WindowConfig,
}

/// What the guest join box holds before anyone has typed in it: this machine,
/// so that two front ends on one computer link without any typing at all.
pub const DEFAULT_LAN_HOST: &str = "127.0.0.1:7064";

/// What the host bind box holds before anyone has typed in it: every interface,
/// which is what a guest on another machine — or through a tunnel — needs.
pub const DEFAULT_LAN_BIND: &str = "0.0.0.0:7064";

impl Default for Settings {
    fn default() -> Self {
        Self {
            recents: Vec::new(),
            view: ViewOptions::default(),
            video: VideoOptions::default(),
            open_panes: Vec::new(),
            limit_framerate: true,
            audio_sync: false,
            speed: crate::speed::DEFAULT,
            cheats_enabled: false,
            volume: 1.0,
            state_dir: None,
            save_dir: None,
            ui_scale: 0.0,
            // The core's picture is pixel art, and a pale theme washes it out.
            dark_theme: true,
            language: crate::i18n::Language::default(),
            lan_host_address: DEFAULT_LAN_HOST.to_owned(),
            lan_bind_address: DEFAULT_LAN_BIND.to_owned(),
            lan: crate::lan::Tuning::default(),
            remote: crate::remote::Tuning::default(),
            bindings: crate::bindings::Bindings::default(),
            window: crate::app::WindowConfig::default(),
        }
    }
}

impl Settings {
    /// Read the settings file, falling back to defaults on anything unreadable
    /// or unparseable — a corrupt file should not stop the emulator starting.
    pub fn load() -> Self {
        Self::load_for(1)
    }

    /// Read the settings belonging to one emulator instance.
    pub fn load_for(instance: u32) -> Self {
        ensure_instance_layout();
        let path = settings_path(instance);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(mut settings) => {
                settings.normalize();
                settings
            }
            Err(e) => {
                log::warn!("ignoring unreadable {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// Write the settings out, reporting but not propagating failure: losing the
    /// recent list matters less than interrupting whatever the user was doing.
    pub fn save(&self) {
        self.save_for(1);
    }

    /// Write the settings belonging to one emulator instance.
    pub fn save_for(&self, instance: u32) {
        ensure_instance_layout();
        let dir = instance_dir(instance);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::error!("cannot create {}: {e}", dir.display());
            return;
        }
        let path = settings_path(instance);
        let json = match serde_json::to_string_pretty(self) {
            Ok(json) => json,
            Err(e) => {
                log::error!("cannot serialize settings: {e}");
                return;
            }
        };
        if let Err(e) = std::fs::write(&path, json) {
            log::error!("cannot write {}: {e}", path.display());
        }
    }

    /// Clamp anything a hand-edited file could put out of range, so a bad
    /// `settings.json` cannot put the front end in a state its own UI could not
    /// produce. `lunaris` does the same to its `config.json`, for the same
    /// reason.
    pub fn normalize(&mut self) {
        self.lan.normalize();
        self.remote.normalize();
        if self.lan_host_address.trim().is_empty() {
            self.lan_host_address = DEFAULT_LAN_HOST.to_owned();
        }
        if self.lan_bind_address.trim().is_empty() {
            self.lan_bind_address = DEFAULT_LAN_BIND.to_owned();
        }
        self.recents.truncate(RECENT_LIMIT);
        self.speed = crate::speed::clamp(self.speed);
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

/// The primary instance directory this front end keeps its files in.
///
/// Beside the executable, so that a copied-out build stays self-contained.
/// Falls back to the working directory if the executable's path is not knowable.
pub fn config_dir() -> PathBuf {
    instance_dir(1)
}

/// The directory containing all per-instance data.
///
/// `./instances`, relative to the working directory — deliberately the same
/// expression `lunaris` itself uses (`lunaris_gui_common::config::Config::
/// INSTANCES_DIR`), so that running the two front ends from one directory puts
/// their instance trees in the same place and a save made under one is where
/// the other looks for it. That is the point of the request: these are two
/// front ends onto the same games, not two unrelated programs.
///
/// Working-directory-relative rather than executable-relative for the same
/// reason: matching `lunaris` matters more than being relocatable, and the two
/// cannot both be had.
pub fn instances_dir() -> PathBuf {
    PathBuf::from(INSTANCES_DIR)
}

/// Where per-instance data lives, as `lunaris` spells it.
pub const INSTANCES_DIR: &str = "./instances";

/// Return the root directory for one emulator instance.
pub fn instance_dir(instance: u32) -> PathBuf {
    instances_dir().join(format!("instance{instance}"))
}

/// Return an instance's dedicated data directory.
pub fn instance_data_dir(instance: u32, kind: &str) -> PathBuf {
    instance_dir(instance).join(kind)
}

/// Create the complete on-disk layout before the first ROM is loaded.
pub fn ensure_instance_layout() {
    for instance in 1..=INSTANCE_COUNT {
        let root = instance_dir(instance);
        for kind in ["saves", "states", "cheats", "logs"] {
            if let Err(error) = std::fs::create_dir_all(root.join(kind)) {
                log::error!("cannot create {}: {error}", root.join(kind).display());
            }
        }
        let settings = root.join("settings.json");
        if !settings.exists()
            && let Ok(json) = serde_json::to_string_pretty(&Settings::default())
            && let Err(error) = std::fs::write(&settings, json)
        {
            log::error!("cannot write {}: {error}", settings.display());
        }
    }
    ensure_translation_templates();
}

/// Write a full translation file for each language, once.
///
/// At startup rather than on exit, because a front end that is closed by the
/// task manager — or that crashes — never reaches `on_exit`, and a user looking
/// for something to edit would find nothing. Existing files are left alone:
/// they are the user's, not ours.
fn ensure_translation_templates() {
    for language in crate::i18n::Language::ALL {
        let path = crate::i18n::I18nMap::i18n_path(*language);
        if path.exists() {
            continue;
        }
        if let Err(error) = crate::i18n::I18nMap::built_in(*language).save_template() {
            log::error!("{error}");
        }
    }
}

fn settings_path(instance: u32) -> PathBuf {
    instance_dir(instance).join("settings.json")
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
