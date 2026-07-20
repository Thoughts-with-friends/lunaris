//! Persisted configuration for the egui front end (`config.json`).
//!
//! Key/gamepad bindings are fixed (not user-remappable) in this first egui
//! implementation — see `docs/design/egui-migration-design.md` §8.2 for the
//! full backend-neutral chord-binding design this intentionally simplifies.
//! Everything else (window geometry, paths, audio, video options) matches
//! the imgui front end's `config.json` shape where applicable.

use std::path::PathBuf;

use crate::framebuffer::ScreenLayout;
use crate::input::enums::{InputBinding, JoystickId};
use crate::input::input_default::default_input_bindings;
use crate::upscale::{self, UpscaleMethod};
use serde::{Deserialize, Serialize};

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
    pub video: VideoConfig,

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
            video: VideoConfig::default(),
            joystick_id: JoystickId::Joystick1,
            input_bindings: default_input_bindings(),
        }
    }
}

impl Config {
    const PATH: &'static str = "./config.json";

    pub fn load() -> Self {
        let mut config: Self = match std::fs::read_to_string(Self::PATH) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        config.video.upscale_factor = upscale::clamp_factor(config.video.upscale_factor);
        config
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::PATH, json);
        }
    }
}
