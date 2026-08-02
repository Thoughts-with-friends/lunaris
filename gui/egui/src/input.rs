use std::collections::HashSet;

use gilrs::Button;
use lunaris_gui_common::input::enums::{
    AxisDirection, BindKey, GamepadAxis, GamepadButton, InputBinding, InputSource, JoystickId,
    KeyboardKey,
};
use nds_core::{
    log::warn,
    nds::{self, NDS},
};

const DEAD_ZONE: f32 = 0.3;

/// NOTE:
/// Physical input devices must NOT directly call
/// `nds.press_key()` / `nds.release_key()`.
///
/// Multiple independent input sources can target the same
/// emulator key during a single frame.
///
/// Example:
///
/// - DPad presses `Down`
/// - Analog stick is neutral
/// - Stick handler releases `Down`
///
/// Final result becomes released even though DPad is pressed.
///
/// To avoid this, all physical device state is first merged
/// into `InputState`.
///
/// The final NDS key state is resolved exactly once per frame
/// through config-defined input bindings.
#[derive(Default)]
pub struct InputState {
    /// Currently pressed keyboard keys.
    pub keyboard: HashSet<KeyboardKey>,

    /// Currently pressed gamepad buttons.
    pub gamepad_buttons: HashSet<GamepadButton>,

    /// Currently active gamepad axis directions.
    pub gamepad_axes: HashSet<(GamepadAxis, AxisDirection)>,
}

impl InputState {
    pub fn is_source_active(&self, source: &InputSource) -> bool {
        match source {
            InputSource::Keyboard { key } => self.keyboard.contains(key),

            InputSource::GamepadButton { button } => self.gamepad_buttons.contains(button),

            InputSource::GamepadAxis { axis, direction } => {
                self.gamepad_axes.contains(&(*axis, *direction))
            }
        }
    }
}

fn apply_key(nds: &mut NDS, pressed: bool, key: nds::Key) {
    if pressed {
        nds.press_key(key);
    } else {
        nds.release_key(key);
    }
}

/// Applies config-defined input bindings.
///
/// Every binding source acts as a logical AND.
///
/// Example:
///
/// - Ctrl + L
/// - LB + RB
///
/// require every source to be active simultaneously.
pub fn apply_input_bindings(nds: &mut NDS, bindings: &[InputBinding], input: &InputState) {
    let mut active_keys = HashSet::new();

    for binding in bindings {
        let active = binding.sources.iter().all(|source| input.is_source_active(source));

        if active {
            active_keys.insert(binding.target);
        }
    }

    // Default keyboard bindings, matching the imgui and egui front end's defaults.
    apply_key(nds, active_keys.contains(&BindKey::Up), nds::Key::Up);
    apply_key(nds, active_keys.contains(&BindKey::Down), nds::Key::Down);
    apply_key(nds, active_keys.contains(&BindKey::Left), nds::Key::Left);
    apply_key(nds, active_keys.contains(&BindKey::Right), nds::Key::Right);
    apply_key(nds, active_keys.contains(&BindKey::A), nds::Key::A);
    apply_key(nds, active_keys.contains(&BindKey::B), nds::Key::B);
    apply_key(nds, active_keys.contains(&BindKey::X), nds::Key::X);
    apply_key(nds, active_keys.contains(&BindKey::Y), nds::Key::Y);
    apply_key(nds, active_keys.contains(&BindKey::L), nds::Key::L);
    apply_key(nds, active_keys.contains(&BindKey::R), nds::Key::R);
    apply_key(nds, active_keys.contains(&BindKey::Start), nds::Key::Start);
    apply_key(nds, active_keys.contains(&BindKey::Select), nds::Key::Select);
}

/// Updates raw keyboard state.
///
/// This function intentionally does NOT know anything
/// about emulator bindings.
pub fn update_keyboard_input(input: &mut InputState, key: egui::Key, pressed: bool) {
    let key = egui_to_config_keyboard_key(key);

    if pressed {
        input.keyboard.insert(key);
    } else {
        input.keyboard.remove(&key);
    }
}

fn update_gamepad_button(input: &mut InputState, state: &gilrs::Gamepad, button: gilrs::Button) {
    let key = egui_to_config_gamepad_button(button);

    if state.is_pressed(button) {
        input.gamepad_buttons.insert(key);
    } else {
        input.gamepad_buttons.remove(&key);
    }
}

/// Resolves the first connected gamepad's state into the set of
/// currently-desired NDS keys (buttons + left-stick-as-D-pad).
fn update_gamepad_axis(input: &mut InputState, state: &gilrs::Gamepad, axis: gilrs::Axis) {
    let value = match axis == gilrs::Axis::LeftStickY || axis == gilrs::Axis::RightStickY {
        true => -state.value(axis),
        false => state.value(axis),
    };

    let (negative, positive) = {
        let axis: GamepadAxis = egui_to_config_axis(axis);
        ((axis, AxisDirection::Negative), (axis, AxisDirection::Positive))
    };

    let mut update_axis = |axis, active: bool| {
        if active {
            input.gamepad_axes.insert(axis);
        } else {
            input.gamepad_axes.remove(&axis);
        }
    };

    update_axis(negative, value < -DEAD_ZONE);
    update_axis(positive, value > DEAD_ZONE);
}

/// Updates raw gamepad state.
///
/// This function intentionally does NOT know anything
/// about emulator bindings.
pub fn update_gamepad_input(
    gilrs: &gilrs::Gilrs,
    input: &mut InputState,
    joystick_id: lunaris_gui_common::input::enums::JoystickId,
) {
    let Some(state) = get_gamepad_by_joystick_id(gilrs, joystick_id) else {
        return;
    };

    use gilrs::Button::*;

    //
    // Buttons
    //
    update_gamepad_button(input, &state, South);
    update_gamepad_button(input, &state, East);
    update_gamepad_button(input, &state, West);
    update_gamepad_button(input, &state, North);

    update_gamepad_button(input, &state, DPadUp);
    update_gamepad_button(input, &state, DPadDown);
    update_gamepad_button(input, &state, DPadLeft);
    update_gamepad_button(input, &state, DPadRight);

    update_gamepad_button(input, &state, LeftTrigger);
    update_gamepad_button(input, &state, RightTrigger);

    update_gamepad_button(input, &state, LeftThumb);
    update_gamepad_button(input, &state, RightThumb);

    update_gamepad_button(input, &state, Start);
    update_gamepad_button(input, &state, Select);
    update_gamepad_button(input, &state, Mode);

    //
    // Axes
    //
    update_gamepad_axis(input, &state, gilrs::Axis::LeftStickX);
    update_gamepad_axis(input, &state, gilrs::Axis::LeftStickY);

    update_gamepad_axis(input, &state, gilrs::Axis::RightStickX);
    update_gamepad_axis(input, &state, gilrs::Axis::RightStickY);

    update_gamepad_axis(input, &state, gilrs::Axis::LeftZ);
    update_gamepad_axis(input, &state, gilrs::Axis::RightZ);
}

fn get_gamepad_by_joystick_id(
    gilrs: &gilrs::Gilrs,
    joystick_id: JoystickId,
) -> Option<gilrs::Gamepad<'_>> {
    let index = match joystick_id {
        JoystickId::Joystick1 => 0,
        JoystickId::Joystick2 => 1,
        JoystickId::Joystick3 => 2,
        JoystickId::Joystick4 => 3,
        JoystickId::Joystick5 => 4,
        JoystickId::Joystick6 => 5,
        JoystickId::Joystick7 => 6,
        JoystickId::Joystick8 => 7,
        JoystickId::Joystick9 => 8,
        JoystickId::Joystick10 => 9,
        JoystickId::Joystick11 => 10,
        JoystickId::Joystick12 => 11,
        JoystickId::Joystick13 => 12,
        JoystickId::Joystick14 => 13,
        JoystickId::Joystick15 => 14,
        JoystickId::Joystick16 => 15,
    };

    gilrs.gamepads().nth(index).map(|pad| pad.1) // Option<(pad.0: GamePadId, pad.1: Gamepad<'_>)>
}

fn egui_to_config_keyboard_key(key: egui::Key) -> lunaris_gui_common::input::enums::KeyboardKey {
    use lunaris_gui_common::input::enums::KeyboardKey as Key;

    match key {
        egui::Key::Space => Key::Space,
        egui::Key::Comma => Key::Comma,
        egui::Key::Minus => Key::Minus,
        egui::Key::Period => Key::Period,
        egui::Key::Slash => Key::Slash,
        egui::Key::Num0 => Key::Num0,
        egui::Key::Num1 => Key::Num1,
        egui::Key::Num2 => Key::Num2,
        egui::Key::Num3 => Key::Num3,
        egui::Key::Num4 => Key::Num4,
        egui::Key::Num5 => Key::Num5,
        egui::Key::Num6 => Key::Num6,
        egui::Key::Num7 => Key::Num7,
        egui::Key::Num8 => Key::Num8,
        egui::Key::Num9 => Key::Num9,
        egui::Key::Semicolon => Key::Semicolon,
        egui::Key::Equals => Key::Equal,
        egui::Key::A => Key::A,
        egui::Key::B => Key::B,
        egui::Key::C => Key::C,
        egui::Key::D => Key::D,
        egui::Key::E => Key::E,
        egui::Key::F => Key::F,
        egui::Key::G => Key::G,
        egui::Key::H => Key::H,
        egui::Key::I => Key::I,
        egui::Key::J => Key::J,
        egui::Key::K => Key::K,
        egui::Key::L => Key::L,
        egui::Key::M => Key::M,
        egui::Key::N => Key::N,
        egui::Key::O => Key::O,
        egui::Key::P => Key::P,
        egui::Key::Q => Key::Q,
        egui::Key::R => Key::R,
        egui::Key::S => Key::S,
        egui::Key::T => Key::T,
        egui::Key::U => Key::U,
        egui::Key::V => Key::V,
        egui::Key::W => Key::W,
        egui::Key::X => Key::X,
        egui::Key::Y => Key::Y,
        egui::Key::Z => Key::Z,
        egui::Key::OpenBracket => Key::LeftBracket,
        egui::Key::Backslash => Key::Backslash,
        egui::Key::CloseBracket => Key::RightBracket,
        egui::Key::Escape => Key::Escape,
        egui::Key::Enter => Key::Enter,
        egui::Key::Tab => Key::Tab,
        egui::Key::Backspace => Key::Backspace,
        egui::Key::Insert => Key::Insert,
        egui::Key::Delete => Key::Delete,
        egui::Key::ArrowRight => Key::Right,
        egui::Key::ArrowLeft => Key::Left,
        egui::Key::ArrowDown => Key::Down,
        egui::Key::ArrowUp => Key::Up,
        egui::Key::PageUp => Key::PageUp,
        egui::Key::PageDown => Key::PageDown,
        egui::Key::Home => Key::Home,
        egui::Key::End => Key::End,
        egui::Key::F1 => Key::F1,
        egui::Key::F2 => Key::F2,
        egui::Key::F3 => Key::F3,
        egui::Key::F4 => Key::F4,
        egui::Key::F5 => Key::F5,
        egui::Key::F6 => Key::F6,
        egui::Key::F7 => Key::F7,
        egui::Key::F8 => Key::F8,
        egui::Key::F9 => Key::F9,
        egui::Key::F10 => Key::F10,
        egui::Key::F11 => Key::F11,
        egui::Key::F12 => Key::F12,
        egui::Key::F13 => Key::F13,
        egui::Key::F14 => Key::F14,
        egui::Key::F15 => Key::F15,
        egui::Key::F16 => Key::F16,
        egui::Key::F17 => Key::F17,
        egui::Key::F18 => Key::F18,
        egui::Key::F19 => Key::F19,
        egui::Key::F20 => Key::F20,
        egui::Key::F21 => Key::F21,
        egui::Key::F22 => Key::F22,
        egui::Key::F23 => Key::F23,
        egui::Key::F24 => Key::F24,
        egui::Key::F25 => Key::F25,
        egui::Key::Copy
        | egui::Key::Cut
        | egui::Key::Paste
        | egui::Key::Colon
        | egui::Key::Pipe
        | egui::Key::Questionmark
        | egui::Key::Exclamationmark
        | egui::Key::OpenCurlyBracket
        | egui::Key::CloseCurlyBracket
        | egui::Key::Backtick
        | egui::Key::Plus
        | egui::Key::Quote
        | egui::Key::F26
        | egui::Key::F27
        | egui::Key::F28
        | egui::Key::F29
        | egui::Key::F30
        | egui::Key::F31
        | egui::Key::F32
        | egui::Key::F33
        | egui::Key::F34
        | egui::Key::F35
        | egui::Key::BrowserBack => Key::Unknown,
    }
}

fn egui_to_config_gamepad_button(button: Button) -> GamepadButton {
    match button {
        Button::South => GamepadButton::ButtonA,
        Button::East => GamepadButton::ButtonB,
        Button::North => GamepadButton::ButtonY,
        Button::West => GamepadButton::ButtonX,
        Button::C => GamepadButton::ButtonGuide,
        Button::Z => GamepadButton::ButtonGuide,
        Button::LeftTrigger => GamepadButton::ButtonLeftBumper,
        Button::LeftTrigger2 => GamepadButton::ButtonLeftBumper,
        Button::RightTrigger => GamepadButton::ButtonRightBumper,
        Button::RightTrigger2 => GamepadButton::ButtonRightBumper,
        Button::Select => GamepadButton::ButtonBack,
        Button::Start => GamepadButton::ButtonStart,
        Button::Mode => GamepadButton::ButtonGuide,
        Button::LeftThumb => GamepadButton::ButtonLeftThumb,
        Button::RightThumb => GamepadButton::ButtonRightThumb,
        Button::DPadUp => GamepadButton::ButtonDpadUp,
        Button::DPadDown => GamepadButton::ButtonDpadDown,
        Button::DPadLeft => GamepadButton::ButtonDpadLeft,
        Button::DPadRight => GamepadButton::ButtonDpadRight,
        Button::Unknown => GamepadButton::ButtonGuide,
    }
}

fn egui_to_config_axis(axis: gilrs::Axis) -> GamepadAxis {
    match axis {
        gilrs::Axis::LeftStickX => GamepadAxis::AxisLeftX,
        gilrs::Axis::LeftStickY => GamepadAxis::AxisLeftY,
        gilrs::Axis::LeftZ => GamepadAxis::AxisLeftTrigger, // unkown
        gilrs::Axis::RightStickX => GamepadAxis::AxisRightX,
        gilrs::Axis::RightStickY => GamepadAxis::AxisRightY,
        gilrs::Axis::RightZ => GamepadAxis::AxisRightTrigger, // unkown
        gilrs::Axis::DPadX => GamepadAxis::AxisRightX,        // unkown
        gilrs::Axis::DPadY => GamepadAxis::AxisRightY,        // unkown
        gilrs::Axis::Unknown => {
            nds_core::log::warn!("unknown axis!");
            GamepadAxis::AxisLeftTrigger
        }
    }
}

pub fn keyboard_keys(input_bindings: &[InputBinding]) -> Vec<(BindKey, egui::Key)> {
    input_bindings
        .iter()
        .flat_map(|binding| {
            binding.sources.iter().filter_map(move |source| match source {
                InputSource::Keyboard { key } => {
                    Some((binding.target, config_to_egui_keyboard_key(*key)))
                }
                _ => None,
            })
        })
        .collect()
}

fn config_to_egui_keyboard_key(key: lunaris_gui_common::input::enums::KeyboardKey) -> egui::Key {
    use lunaris_gui_common::input::enums::KeyboardKey as Key;

    match key {
        Key::Space => egui::Key::Space,
        Key::Comma => egui::Key::Comma,
        Key::Minus => egui::Key::Minus,
        Key::Period => egui::Key::Period,
        Key::Slash => egui::Key::Slash,
        Key::Num0 => egui::Key::Num0,
        Key::Num1 => egui::Key::Num1,
        Key::Num2 => egui::Key::Num2,
        Key::Num3 => egui::Key::Num3,
        Key::Num4 => egui::Key::Num4,
        Key::Num5 => egui::Key::Num5,
        Key::Num6 => egui::Key::Num6,
        Key::Num7 => egui::Key::Num7,
        Key::Num8 => egui::Key::Num8,
        Key::Num9 => egui::Key::Num9,
        Key::Semicolon => egui::Key::Semicolon,
        Key::Equal => egui::Key::Equals,
        Key::A => egui::Key::A,
        Key::B => egui::Key::B,
        Key::C => egui::Key::C,
        Key::D => egui::Key::D,
        Key::E => egui::Key::E,
        Key::F => egui::Key::F,
        Key::G => egui::Key::G,
        Key::H => egui::Key::H,
        Key::I => egui::Key::I,
        Key::J => egui::Key::J,
        Key::K => egui::Key::K,
        Key::L => egui::Key::L,
        Key::M => egui::Key::M,
        Key::N => egui::Key::N,
        Key::O => egui::Key::O,
        Key::P => egui::Key::P,
        Key::Q => egui::Key::Q,
        Key::R => egui::Key::R,
        Key::S => egui::Key::S,
        Key::T => egui::Key::T,
        Key::U => egui::Key::U,
        Key::V => egui::Key::V,
        Key::W => egui::Key::W,
        Key::X => egui::Key::X,
        Key::Y => egui::Key::Y,
        Key::Z => egui::Key::Z,
        Key::LeftBracket => egui::Key::OpenBracket,
        Key::Backslash => egui::Key::Backslash,
        Key::RightBracket => egui::Key::CloseBracket,
        Key::Escape => egui::Key::Escape,
        Key::Enter => egui::Key::Enter,
        Key::Tab => egui::Key::Tab,
        Key::Backspace => egui::Key::Backspace,
        Key::Insert => egui::Key::Insert,
        Key::Delete => egui::Key::Delete,
        Key::Right => egui::Key::ArrowRight,
        Key::Left => egui::Key::ArrowLeft,
        Key::Down => egui::Key::ArrowDown,
        Key::Up => egui::Key::ArrowUp,
        Key::PageUp => egui::Key::PageUp,
        Key::PageDown => egui::Key::PageDown,
        Key::Home => egui::Key::Home,
        Key::End => egui::Key::End,
        Key::F1 => egui::Key::F1,
        Key::F2 => egui::Key::F2,
        Key::F3 => egui::Key::F3,
        Key::F4 => egui::Key::F4,
        Key::F5 => egui::Key::F5,
        Key::F6 => egui::Key::F6,
        Key::F7 => egui::Key::F7,
        Key::F8 => egui::Key::F8,
        Key::F9 => egui::Key::F9,
        Key::F10 => egui::Key::F10,
        Key::F11 => egui::Key::F11,
        Key::F12 => egui::Key::F12,
        Key::F13 => egui::Key::F13,
        Key::F14 => egui::Key::F14,
        Key::F15 => egui::Key::F15,
        Key::F16 => egui::Key::F16,
        Key::F17 => egui::Key::F17,
        Key::F18 => egui::Key::F18,
        Key::F19 => egui::Key::F19,
        Key::F20 => egui::Key::F20,
        Key::F21 => egui::Key::F21,
        Key::F22 => egui::Key::F22,
        Key::F23 => egui::Key::F23,
        Key::F24 => egui::Key::F24,
        Key::F25 => egui::Key::F25,

        _ => {
            warn!("unsupported key: {key:?}");
            egui::Key::Space
        }
    }
}
