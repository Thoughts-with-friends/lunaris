//! What the front end remembers between runs, and where it keeps it.
//!
//! melonDS stores its configuration in a directory of its own, which its File
//! menu can open; this is the equivalent. Deliberately small: only the things a
//! user would be annoyed to set twice.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

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
        // A relative save or state directory is one an older version wrote
        // before `instances_dir` was anchored to the executable: it means
        // "wherever this happened to be started from", which is the very thing
        // that used to scatter saves across two trees. Dropped rather than
        // resolved, so it falls back to this instance's own directory -- a
        // directory the user actually chose is always absolute, because it came
        // from a folder dialog.
        self.save_dir = self.save_dir.take().filter(|dir| dir.is_absolute());
        self.state_dir = self.state_dir.take().filter(|dir| dir.is_absolute());
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

/// The directory containing all per-instance data: saves, savestates, cheats,
/// logs and `settings.json`.
///
/// # Why this is not simply `./instances`
///
/// It was, to match the expression `lunaris` itself uses
/// (`lunaris_gui_common::config::Config::INSTANCES_DIR`) so that both front ends
/// put their instance trees in the same place. But that path is relative to the
/// *working directory*, and the working directory is not a property of the
/// program — it is whatever launched it. `cargo run` sets it to the workspace
/// root; double-clicking the executable sets it to the folder the executable is
/// in.
///
/// So one build could hold two complete instance trees at once and silently
/// pick between them by how it was started: a save written under one launch was
/// invisible under the other, and a savestate written from the menu landed in a
/// directory the next run never looked at. That is a data-loss bug wearing the
/// clothes of a path constant.
///
/// # What it is instead
///
/// Resolved once, to an absolute path, from the *executable*, which does not
/// move between launches:
///
/// 1. `MELON_EGUI_INSTANCES`, when it is set — the way to put the tree
///    anywhere, including on the same directory `lunaris` uses.
/// 2. For a build inside `target/<profile>/`, the workspace root above it. A
///    development build then keeps one tree however it was started, and it is
///    the same `./instances` a `cargo run` from the workspace root has always
///    used.
/// 3. Otherwise beside the executable, so that a copied-out build is
///    self-contained.
/// 4. `./instances`, if the executable's own path cannot be read at all.
pub fn instances_dir() -> PathBuf {
    static RESOLVED: OnceLock<PathBuf> = OnceLock::new();
    RESOLVED.get_or_init(resolve_instances_dir).clone()
}

/// [`instances_dir`]'s rules, in order. Separate so the reasoning is readable
/// and so it can be tested against a path without running from one.
fn resolve_instances_dir() -> PathBuf {
    if let Some(from_env) = std::env::var_os(INSTANCES_ENV).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(from_env);
    }
    match std::env::current_exe().ok().and_then(|exe| app_root_for(&exe)) {
        Some(root) => root.join("instances"),
        None => PathBuf::from(INSTANCES_DIR),
    }
}

/// Where the tree belongs for an executable at `exe`: the workspace root when
/// it is a `target/<profile>/` build, and the executable's own directory
/// otherwise.
fn app_root_for(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    // `target/debug/melon_egui.exe` and `target/release/…` are the two shapes
    // cargo produces; `target/<triple>/<profile>/…` is the cross-compiled one.
    let mut ancestors = dir.ancestors().skip(1).take(2);
    if let Some(root) = ancestors.find_map(|above| {
        (above.file_name()? == "target").then(|| above.parent().map(Path::to_path_buf))?
    }) {
        return Some(root);
    }
    Some(dir.to_path_buf())
}

/// The environment variable that overrides where the instance tree lives.
pub const INSTANCES_ENV: &str = "MELON_EGUI_INSTANCES";

/// Where per-instance data lives when the executable's path is unknowable, as
/// `lunaris` spells it.
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

    use super::{RECENT_LIMIT, Settings, app_root_for};

    /// A development build must find the workspace's own tree however it was
    /// started -- this is the bug that split saves across two directories.
    #[test]
    fn a_target_build_anchors_on_the_workspace_root() {
        let root = Path::new("D:/work/lunaris");
        assert_eq!(
            app_root_for(&root.join("target/release/melon_egui.exe")),
            Some(root.to_path_buf()),
        );
        assert_eq!(
            app_root_for(&root.join("target/debug/melon_egui.exe")),
            Some(root.to_path_buf()),
        );
        // The cross-compiled shape, `target/<triple>/<profile>/`.
        assert_eq!(
            app_root_for(&root.join("target/x86_64-pc-windows-msvc/release/melon_egui.exe")),
            Some(root.to_path_buf()),
        );
    }

    /// A build copied out somewhere keeps its data beside itself.
    #[test]
    fn a_distributed_build_anchors_beside_the_executable() {
        let dir = Path::new("D:/Games/melon_egui");
        assert_eq!(app_root_for(&dir.join("melon_egui.exe")), Some(dir.to_path_buf()));
    }

    /// A directory that merely *contains* the word is not a cargo target dir.
    #[test]
    fn only_a_real_target_directory_counts() {
        let dir = Path::new("D:/Games/targeting/release");
        assert_eq!(app_root_for(&dir.join("melon_egui.exe")), Some(dir.to_path_buf()));
    }

    /// The stale relative directories older versions wrote mean "wherever this
    /// was started from", which is what has to stop being trusted.
    #[test]
    fn a_relative_save_directory_is_dropped_on_load() {
        let mut settings = Settings {
            save_dir: Some(PathBuf::from("./instances/instance1/saves")),
            state_dir: Some(PathBuf::from("D:/absolute/states")),
            ..Settings::default()
        };
        settings.normalize();
        assert_eq!(settings.save_dir, None, "a relative directory falls back to the default");
        assert_eq!(
            settings.state_dir,
            Some(PathBuf::from("D:/absolute/states")),
            "a directory the user chose is absolute and is kept",
        );
    }

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
