use serde::{Deserialize, Serialize};

use std::fs;
use std::path::PathBuf;

/// Analog axis direction.
///
/// Positive:
/// - Right
/// - Down
/// - Trigger pressed
///
/// Negative:
/// - Left
/// - Up
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisDirection {
    Positive,
    Negative,
}

/// Single physical input source.
///
/// This intentionally stores GLFW enums directly because:
///
/// - GLFW already supports serde
/// - The emulator backend is GLFW-specific
/// - GLFW enums are stable and readable
///
/// Multiple sources may be combined into a chord
/// through `InputBinding.sources`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputSource {
    /// Keyboard key input.
    Keyboard { key: glfw::Key },

    /// Digital gamepad button input.
    GamepadButton {
        joystick: glfw::JoystickId,
        button: glfw::GamepadButton,
    },

    /// Analog gamepad axis input.
    ///
    /// `direction` selects which half of the axis
    /// is considered active.
    GamepadAxis {
        joystick: glfw::JoystickId,
        axis: glfw::GamepadAxis,
        direction: AxisDirection,
    },
}

/// Emulator logical input.
///
/// These are independent from keyboard/gamepad APIs
/// and represent emulator-side actions only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindKey {
    Up,
    Down,
    Left,
    Right,

    A,
    B,
    X,
    Y,

    L,
    R,

    Start,
    Select,
}

/// Input binding definition.
///
/// `sources` acts as a logical AND.
///
/// Examples:
///
/// - Ctrl + L
/// - LB + RB
/// - Shift + DPadUp
///
/// Every source must be active simultaneously.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputBinding {
    pub sources: Vec<InputSource>,
    pub target: BindKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    pub pos_x: i32,
    pub pos_y: i32,
    pub width: i32,
    pub height: i32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            pos_x: 100,
            pos_y: 100,
            width: 512,
            height: 768,
        }
    }
}

/// Emulator configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub bios7_path: Option<PathBuf>,
    pub bios9_path: Option<PathBuf>,
    pub firmware_path: Option<PathBuf>,
    pub last_rom_path: Option<PathBuf>,

    pub window: WindowConfig,

    pub audio_volume: f32,

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
        use glfw::GamepadAxis::*;
        use glfw::GamepadButton::*;
        use glfw::JoystickId::*;
        use glfw::Key::*;

        Self {
            bios7_path: None,
            bios9_path: None,
            firmware_path: None,
            last_rom_path: None,
            window: WindowConfig::default(),
            audio_volume: 100.0,

            input_bindings: vec![
                //
                // Keyboard
                //
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: Up }],
                    target: BindKey::Up,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: Down }],
                    target: BindKey::Down,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: Left }],
                    target: BindKey::Left,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: Right }],
                    target: BindKey::Right,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: A }],
                    target: BindKey::A,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: B }],
                    target: BindKey::B,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: X }],
                    target: BindKey::X,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: Y }],
                    target: BindKey::Y,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: L }],
                    target: BindKey::L,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: R }],
                    target: BindKey::R,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: T }],
                    target: BindKey::Start,
                },
                InputBinding {
                    sources: vec![InputSource::Keyboard { key: E }],
                    target: BindKey::Select,
                },
                //
                // Gamepad buttons
                //
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonDpadUp,
                    }],
                    target: BindKey::Up,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonDpadDown,
                    }],
                    target: BindKey::Down,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonDpadLeft,
                    }],
                    target: BindKey::Left,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonDpadRight,
                    }],
                    target: BindKey::Right,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonB,
                    }],
                    target: BindKey::A,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonA,
                    }],
                    target: BindKey::B,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonY,
                    }],
                    target: BindKey::X,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonX,
                    }],
                    target: BindKey::Y,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonLeftBumper,
                    }],
                    target: BindKey::L,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonRightBumper,
                    }],
                    target: BindKey::R,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonStart,
                    }],
                    target: BindKey::Start,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadButton {
                        joystick: Joystick1,
                        button: ButtonBack,
                    }],
                    target: BindKey::Select,
                },
                //
                // Left stick
                //
                InputBinding {
                    sources: vec![InputSource::GamepadAxis {
                        joystick: Joystick1,
                        axis: AxisLeftX,
                        direction: AxisDirection::Negative,
                    }],
                    target: BindKey::Left,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadAxis {
                        joystick: Joystick1,
                        axis: AxisLeftX,
                        direction: AxisDirection::Positive,
                    }],
                    target: BindKey::Right,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadAxis {
                        joystick: Joystick1,
                        axis: AxisLeftY,
                        direction: AxisDirection::Negative,
                    }],
                    target: BindKey::Up,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadAxis {
                        joystick: Joystick1,
                        axis: AxisLeftY,
                        direction: AxisDirection::Positive,
                    }],
                    target: BindKey::Down,
                },
                //
                // Triggers
                //
                InputBinding {
                    sources: vec![InputSource::GamepadAxis {
                        joystick: Joystick1,
                        axis: AxisLeftTrigger,
                        direction: AxisDirection::Positive,
                    }],
                    target: BindKey::L,
                },
                InputBinding {
                    sources: vec![InputSource::GamepadAxis {
                        joystick: Joystick1,
                        axis: AxisRightTrigger,
                        direction: AxisDirection::Positive,
                    }],
                    target: BindKey::R,
                },
            ],
        }
    }
}

impl Config {
    const PATH: &'static str = "./config.json";

    pub fn load() -> Self {
        match fs::read_to_string(Self::PATH) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let json = serde_json::to_string_pretty(self).unwrap();
        fs::write(Self::PATH, json).unwrap();
    }
}
