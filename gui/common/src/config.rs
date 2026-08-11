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

impl LanConfig {
    /// The Wi-Fi MAC suffix actually baked into this **process's**
    /// firmware image, combining the persisted [`LanConfig::mac_suffix`]
    /// with this process's OS process id.
    ///
    /// `Config::PATH` (`"./config.json"`) is resolved relative to the
    /// working directory, so two `lunaris` processes launched from the same
    /// directory -- the ordinary way to test local multiplayer on one
    /// PC -- load the *same* `config.json` and therefore the *same*
    /// persisted `mac_suffix`. Without this, both processes would patch
    /// their firmware to an identical MAC address despite
    /// `docs/design/review_mp_local.md` F13's fix, and the DS association
    /// handshake would never complete (two "different" consoles with the
    /// same MAC look like one console to the Wi-Fi hardware). Mixing in
    /// `std::process::id()` -- guaranteed unique among concurrently running
    /// processes on this machine, which is exactly the property needed
    /// here -- fixes that without requiring the user to hand-edit
    /// `config.json` between launches.
    ///
    /// Must be used consistently everywhere a MAC-derived value has to
    /// match what's actually in this process's firmware: both
    /// `gui/common/src/loader.rs`'s firmware patch and
    /// `RoomConfig::mac_suffix` (so the room's MAC-collision check compares
    /// the values actually in use, not the raw persisted one).
    #[must_use]
    pub fn effective_mac_suffix(&self) -> [u8; 3] {
        mix_mac_suffix(self.mac_suffix, std::process::id())
    }
}

/// Pure XOR mix of a persisted suffix with a process id, factored out of
/// [`LanConfig::effective_mac_suffix`] so it's testable without depending
/// on the calling test's own live `std::process::id()`.
fn mix_mac_suffix(suffix: [u8; 3], pid: u32) -> [u8; 3] {
    [
        suffix[0] ^ (pid & 0xFF) as u8,
        suffix[1] ^ ((pid >> 8) & 0xFF) as u8,
        suffix[2] ^ ((pid >> 16) & 0xFF) as u8,
    ]
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
            last_host_ip: String::new(),
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

/// Slowest selectable emulation speed (half of native).
pub const MIN_EMU_SPEED: f32 = 0.5;

/// Fastest selectable emulation speed. Actual throughput is still bounded by
/// how many frames the host can emulate per repaint.
pub const MAX_EMU_SPEED: f32 = 10.0;

impl Config {
    const PATH: &'static str = "./config.json";

    pub fn load() -> Self {
        let mut config: Self = match std::fs::read_to_string(Self::PATH) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        config.video.upscale_factor = upscale::clamp_factor(config.video.upscale_factor);
        config.emu_speed = if config.emu_speed.is_finite() {
            config.emu_speed.clamp(MIN_EMU_SPEED, MAX_EMU_SPEED)
        } else {
            1.0
        };
        config.lan.max_players = config.lan.max_players.clamp(1, 16);
        config.lan.runahead_us = config.lan.runahead_us.clamp(250, 16_000);
        config.lan.recv_timeout_ms = config.lan.recv_timeout_ms.clamp(2, 40);
        config
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

    #[test]
    fn mac_suffix_mixing_differs_for_different_process_ids() {
        // Two processes sharing the same persisted `mac_suffix` (the
        // ordinary case when both load the same `config.json`) must still
        // end up with different effective suffixes.
        let base = [1, 2, 3];
        assert_ne!(mix_mac_suffix(base, 1000), mix_mac_suffix(base, 1004));
    }

    #[test]
    fn mac_suffix_mixing_is_identity_for_pid_zero() {
        assert_eq!(mix_mac_suffix([9, 8, 7], 0), [9, 8, 7]);
    }

    #[test]
    fn mac_suffix_mixing_is_deterministic_for_one_process() {
        let base = [4, 5, 6];
        assert_eq!(mix_mac_suffix(base, 42), mix_mac_suffix(base, 42));
    }
}
