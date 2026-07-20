// SPDX-FileCopyrightText: (C) 2026 The glfw-rs developers.
// SPDX-License-Identifier: Apache-2.0
//
// ref: glfw v0.62.0 lib.rs

/// Input keys.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum KeyboardKey {
    Space,
    Apostrophe,
    Comma,
    Minus,
    Period,
    Slash,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Semicolon,
    Equal,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    LeftBracket,
    Backslash,
    RightBracket,
    GraveAccent,
    World1,
    World2,

    Escape,
    Enter,
    Tab,
    Backspace,
    Insert,
    Delete,
    Right,
    Left,
    Down,
    Up,
    PageUp,
    PageDown,
    Home,
    End,
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    F25,
    Kp0,
    Kp1,
    Kp2,
    Kp3,
    Kp4,
    Kp5,
    Kp6,
    Kp7,
    Kp8,
    Kp9,
    KpDecimal,
    KpDivide,
    KpMultiply,
    KpSubtract,
    KpAdd,
    KpEnter,
    KpEqual,
    LeftShift,
    LeftControl,
    LeftAlt,
    LeftSuper,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    Menu,
    Unknown,
}

/// Button identifier tokens.
#[repr(i32)]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum GamepadButton {
    ButtonA,
    ButtonB,
    ButtonX,
    ButtonY,
    ButtonLeftBumper,
    ButtonRightBumper,
    ButtonBack,
    ButtonStart,
    ButtonGuide,
    ButtonLeftThumb,
    ButtonRightThumb,
    ButtonDpadUp,
    ButtonDpadRight,
    ButtonDpadDown,
    ButtonDpadLeft,
}

/// Axis identifier tokens.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum GamepadAxis {
    AxisLeftX,
    AxisLeftY,
    AxisRightX,
    AxisRightY,
    AxisLeftTrigger,
    AxisRightTrigger,
}

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisDirection {
    Positive,
    Negative,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputSource {
    /// Keyboard key input.
    Keyboard { key: KeyboardKey },

    /// Digital gamepad button input.
    GamepadButton { button: GamepadButton },

    /// Analog gamepad axis input.
    ///
    /// `direction` selects which half of the axis
    /// is considered active.
    GamepadAxis { axis: GamepadAxis, direction: AxisDirection },
}

/// Emulator logical input.
///
/// These are independent from keyboard/gamepad APIs
/// and represent emulator-side actions only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InputBinding {
    pub sources: Vec<InputSource>,
    pub target: BindKey,
}

/// Joystick identifier tokens.
#[repr(i32)]
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Debug,
    serde::Serialize,
    serde::Deserialize
)]
pub enum JoystickId {
    Joystick1,
    Joystick2,
    Joystick3,
    Joystick4,
    Joystick5,
    Joystick6,
    Joystick7,
    Joystick8,
    Joystick9,
    Joystick10,
    Joystick11,
    Joystick12,
    Joystick13,
    Joystick14,
    Joystick15,
    Joystick16,
}
