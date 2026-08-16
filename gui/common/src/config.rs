//! Persisted configuration for the egui front end (`config.json`).
//!
//! Key/gamepad bindings live in [`Config::input_bindings`] and are editable
//! from the front end's "Input Settings" window (one keyboard key and one
//! gamepad button per NDS button); analog-axis and chord bindings remain
//! hand-edited here — see `docs/design/egui-migration-design.md` §8.2 for the
//! full backend-neutral chord-binding design this intentionally simplifies.
//! Everything else (window geometry, paths, audio, video options) matches
//! the imgui front end's `config.json` shape where applicable.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    framebuffer::ScreenLayout,
    input::{
        enums::{InputBinding, JoystickId},
        input_default::default_input_bindings,
    },
    upscale::{self, UpscaleMethod},
};

/// Host address the guest "Join Room" field is pre-filled with. Loopback,
/// i.e. a second `lunaris` process on this same machine, which is the usual
/// way local wireless play is tested.
pub const DEFAULT_HOST_IP: &str = "127.0.0.1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    /// X coordinate of the window's top-left corner (outer rect).
    pub pos_x: f32,

    /// Y coordinate of the window's top-left corner (outer rect).
    pub pos_y: f32,

    /// Inner width of the window (excludes OS decorations).
    pub width: f32,

    /// Inner height of the window (excludes title bar and OS decorations).
    pub height: f32,

    /// Whether the window was maximized when the application last closed.
    pub maximized: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { pos_x: 100.0, pos_y: 100.0, width: 512.0, height: 768.0, maximized: false }
    }
}

/// Texture minification/magnification filter for the screen textures.
///
/// See `docs/design/egui-migration-design.md` §7.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenFilter {
    #[default]
    Nearest,
    Linear,
}

/// Video quality options. See `docs/design/egui-migration-design.md` §7.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    pub filter: ScreenFilter,
    pub screen_layout: ScreenLayout,
    pub screen_gap: f32,
    pub integer_scaling: bool,
    pub show_fps_overlay: bool,
    /// See `docs/design/resolution-upscaling-design.md` §4 and §7.
    pub upscale_method: UpscaleMethod,
    /// Integer scale factor in `upscale::MIN_FACTOR..=upscale::MAX_FACTOR`,
    /// clamped on load in [`Config::load`] since this file is hand-editable.
    pub upscale_factor: u8,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            filter: ScreenFilter::default(),
            screen_layout: ScreenLayout::default(),
            screen_gap: 0.0,
            integer_scaling: false,
            show_fps_overlay: false,
            upscale_method: UpscaleMethod::default(),
            upscale_factor: 2,
        }
    }
}

/// Local multiplayer room settings. See
/// `docs/design/design_lan.md` §12.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LanConfig {
    pub player_name: String,
    pub room_name: String,
    /// Host address the guest join field is pre-filled with. Defaults to
    /// [`DEFAULT_HOST_IP`] so the common case -- two `lunaris` processes on
    /// one machine -- needs no typing.
    pub last_host_ip: String,
    pub control_port: u16,
    pub mp_port: u16,
    pub max_players: u8,
    /// Randomized on first run and persisted, so repeated launches on one
    /// machine keep a stable (and, across machines, very likely unique)
    /// Wi-Fi MAC low-3-bytes suffix. See
    /// `docs/design/design_lan.md` §7.3.
    pub mac_suffix: [u8; 3],
    pub link_auto: bool,
    pub runahead_us: u32,
    pub recv_timeout_ms: u16,
}

impl Default for LanConfig {
    fn default() -> Self {
        // A random-ish default seeded from the current time so two fresh
        // installs on a LAN don't collide; `Config::load` doesn't
        // otherwise touch an RNG dependency for this one field.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u32)
            .unwrap_or(0);
        Self {
            player_name: "Luna".to_owned(),
            room_name: "Lunaris Room".to_owned(),
            last_host_ip: DEFAULT_HOST_IP.to_owned(),
            control_port: 7064,
            mp_port: 7065,
            max_players: 8,
            mac_suffix: [
                (seed & 0xFF) as u8,
                ((seed >> 8) & 0xFF) as u8,
                ((seed >> 16) & 0xFF) as u8,
            ],
            link_auto: true,
            runahead_us: 1000,
            recv_timeout_ms: 8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub bios7_path: Option<PathBuf>,
    pub bios9_path: Option<PathBuf>,
    pub firmware_path: Option<PathBuf>,
    pub last_rom_path: Option<PathBuf>,
    pub save_dir: PathBuf,
    pub save_state_dir: PathBuf,
    pub enable_cheats: bool,
    pub cheat_dir: PathBuf,
    /// Where this instance's log files are written. Set from the instance
    /// directory by `Config::load_for_instance`.
    #[serde(default = "default_log_dir")]
    pub log_dir: PathBuf,
    pub window: WindowConfig,
    pub audio_volume: f32,
    /// Emulation speed multiplier in [`MIN_EMU_SPEED`]..=[`MAX_EMU_SPEED`],
    /// clamped on load since this file is hand-editable. 1.0 is native speed.
    pub emu_speed: f32,
    pub video: VideoConfig,
    pub lan: LanConfig,

    /// Joystick ID for gamepad input.
    pub joystick_id: JoystickId,
    /// Input binding configuration.
    ///
    /// ```no_run
    /// // Example chord:
    /// // Ctrl + L -> Start
    /// InputBinding {
    ///     sources: vec![
    ///         InputSource::Keyboard { key: LeftControl },
    ///         InputSource::Keyboard { key: L },
    ///     ],
    ///     target: BindKey::Start,
    /// },
    /// ```
    pub input_bindings: Vec<InputBinding>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bios7_path: None,
            bios9_path: None,
            firmware_path: None,
            last_rom_path: None,
            save_dir: PathBuf::from("./saves"),
            save_state_dir: PathBuf::from("./states"),
            enable_cheats: false,
            cheat_dir: PathBuf::from("./cheats"),
            log_dir: default_log_dir(),
            window: WindowConfig::default(),
            audio_volume: 100.0,
            emu_speed: 1.0,
            video: VideoConfig::default(),
            lan: LanConfig::default(),
            joystick_id: JoystickId::Joystick1,
            input_bindings: default_input_bindings(),
        }
    }
}

/// Default log destination for a configuration that predates per-instance
/// directories, matching where the logger has always written.
fn default_log_dir() -> PathBuf {
    PathBuf::from("./logs")
}

/// Recursively copies `from` into `to`, never overwriting an existing file.
///
/// Used once, to adopt a pre-per-instance layout's user data. Skipping existing
/// files means re-running it can only ever fill gaps, so a partially-completed
/// first run resumes cleanly instead of clobbering whatever the user has since
/// played.
fn copy_dir_contents(from: &std::path::Path, to: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(from) else { return };
    let _ = std::fs::create_dir_all(to);
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir_contents(&src, &dst);
        } else if !dst.exists() {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}

/// Slowest selectable emulation speed (half of native).
pub const MIN_EMU_SPEED: f32 = 0.5;

/// Fastest selectable emulation speed. Actual throughput is still bounded by
/// how many frames the host can emulate per repaint.
pub const MAX_EMU_SPEED: f32 = 10.0;

impl Config {
    const PATH: &'static str = "./config.json";

    /// Root of the per-instance directory tree.
    ///
    /// Each emulator instance owns `instances/instance<N>/`, holding its own
    /// `config.json`, `saves/`, `states/` and `cheats/`. Instances are numbered
    /// from **1** (the main window), so the Thread Mode guest is `instance2` —
    /// the directory names read the way a person counts, not the way the code
    /// indexes.
    pub const INSTANCES_DIR: &'static str = "./instances";

    /// The directory instance `instance` (0-based) keeps its files in.
    #[must_use]
    pub fn instance_dir(instance: u8) -> PathBuf {
        PathBuf::from(Self::INSTANCES_DIR).join(format!("instance{}", instance as u16 + 1))
    }

    /// `config.json` for instance `instance` (0-based).
    #[must_use]
    pub fn instance_config_path(instance: u8) -> PathBuf {
        Self::instance_dir(instance).join("config.json")
    }

    /// Loads instance `instance`'s configuration, creating its directory tree
    /// on first use.
    ///
    /// A missing per-instance `config.json` is seeded from the legacy top-level
    /// `./config.json` when one exists, so an established setup keeps its BIOS
    /// paths, key bindings and last ROM instead of silently reverting to
    /// defaults. Only the *paths* are then redirected into the instance
    /// directory.
    #[must_use]
    pub fn load_for_instance(instance: u8) -> Self {
        let dir = Self::instance_dir(instance);
        let path = Self::instance_config_path(instance);
        let first_run = !path.exists();
        let _ = std::fs::create_dir_all(&dir);

        let legacy = Self::load();
        let mut config: Self = match std::fs::read_to_string(&path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            // Fall back to the legacy shared config for its non-path settings.
            Err(_) => legacy.clone(),
        };
        config.apply_instance_paths(instance);
        config.normalize();

        // Instance 1 inherits an established single-instance setup's files.
        // Without this the emulator would silently start looking in an empty
        // `instances/instance1/saves`, and a user with years of saves in
        // `./saves` would conclude the update ate them.
        if first_run && instance == 0 {
            config.adopt_legacy_user_data(&legacy);
            config.save_for_instance(instance);
        }
        config
    }

    /// Copies an established single-instance layout's saves, savestates and
    /// cheats into this instance's directories, on first run only.
    ///
    /// **Copies rather than moves**, and only into destinations that do not
    /// already exist. The originals stay put, so a user who dislikes the new
    /// layout — or who runs an older build — still has them. Copying is
    /// deliberate: a move that half-completed would leave the data in neither
    /// place.
    fn adopt_legacy_user_data(&self, legacy: &Self) {
        for (from, to) in [
            (&legacy.save_dir, &self.save_dir),
            (&legacy.save_state_dir, &self.save_state_dir),
            (&legacy.cheat_dir, &self.cheat_dir),
        ] {
            if from != to {
                copy_dir_contents(from, to);
            }
        }
    }

    /// Points the save / savestate / cheat directories at this instance's own
    /// subtree.
    ///
    /// Applied on every load rather than only on creation: these three are
    /// derived from the instance, not user preferences, and letting a
    /// hand-edited or copied `config.json` aim them at another instance's
    /// directory is exactly how two emulators come to overwrite one another's
    /// saves.
    fn apply_instance_paths(&mut self, instance: u8) {
        let dir = Self::instance_dir(instance);
        self.save_dir = dir.join("saves");
        self.save_state_dir = dir.join("states");
        self.cheat_dir = dir.join("cheats");
        self.log_dir = dir.join("logs");
        for sub in [&self.save_dir, &self.save_state_dir, &self.cheat_dir, &self.log_dir] {
            let _ = std::fs::create_dir_all(sub);
        }
    }

    /// Writes this configuration back to instance `instance`'s `config.json`.
    pub fn save_for_instance(&self, instance: u8) {
        let path = Self::instance_config_path(instance);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn load() -> Self {
        let mut config: Self = match std::fs::read_to_string(Self::PATH) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        config.normalize();
        config
    }

    /// Clamps every field with a valid range, so a hand-edited or
    /// version-skewed `config.json` cannot put the emulator in a state its own
    /// UI could not produce.
    fn normalize(&mut self) {
        let config = self;
        config.video.upscale_factor = upscale::clamp_factor(config.video.upscale_factor);
        config.emu_speed = if config.emu_speed.is_finite() {
            config.emu_speed.clamp(MIN_EMU_SPEED, MAX_EMU_SPEED)
        } else {
            1.0
        };
        config.lan.max_players = config.lan.max_players.clamp(1, 16);
        config.lan.runahead_us = config.lan.runahead_us.clamp(250, 16_000);
        config.lan.recv_timeout_ms = config.lan.recv_timeout_ms.clamp(2, 40);
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::PATH, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adopting a legacy layout must never overwrite what the new instance has
    /// already accumulated: re-running the migration can only fill gaps.
    /// Anything else risks a stale `./saves` copy stamping over real progress.
    #[test]
    fn copying_legacy_data_never_overwrites_existing_files() {
        let root = std::env::temp_dir().join(format!("lunaris_cfg_test_{}", std::process::id()));
        let (from, to) = (root.join("from"), root.join("to"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(from.join("nested")).unwrap();
        std::fs::create_dir_all(&to).unwrap();

        std::fs::write(from.join("keep.sav"), b"old").unwrap();
        std::fs::write(from.join("nested/deep.sav"), b"deep").unwrap();
        std::fs::write(to.join("keep.sav"), b"new").unwrap();

        copy_dir_contents(&from, &to);

        assert_eq!(std::fs::read(to.join("keep.sav")).unwrap(), b"new", "existing file survives");
        assert_eq!(
            std::fs::read(to.join("nested/deep.sav")).unwrap(),
            b"deep",
            "missing files are copied, recursively"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
