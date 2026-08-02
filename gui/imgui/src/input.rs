use std::collections::HashSet;

use glfw::GamepadButton::*;
use lunaris_gui_common::input::enums::{
    AxisDirection, BindKey, GamepadAxis, GamepadButton, InputBinding, InputSource, KeyboardKey,
};
use nds_core::nds::{self, NDS};

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
pub fn update_keyboard_input(input: &mut InputState, key: glfw::Key, pressed: bool) {
    let key = glfw_to_config_keyboard_key(key);

    if pressed {
        input.keyboard.insert(key);
    } else {
        input.keyboard.remove(&key);
    }
}

fn update_gamepad_button(
    input: &mut InputState,
    state: &glfw::GamepadState,
    button: glfw::GamepadButton,
) {
    let key = glfw_to_config_gamepad_button(button);

    if matches!(state.get_button_state(button), glfw::Action::Press) {
        input.gamepad_buttons.insert(key);
    } else {
        input.gamepad_buttons.remove(&key);
    }
}

fn update_gamepad_axis(
    input: &mut InputState,
    state: &glfw::GamepadState,
    axis: glfw::GamepadAxis,
) {
    let value = state.get_axis(axis);
    let axis = glfw_to_config_axis(axis);

    let negative = (axis, AxisDirection::Negative);
    let positive = (axis, AxisDirection::Positive);

    if value < -DEAD_ZONE {
        input.gamepad_axes.insert(negative);
    } else {
        input.gamepad_axes.remove(&negative);
    }

    if value > DEAD_ZONE {
        input.gamepad_axes.insert(positive);
    } else {
        input.gamepad_axes.remove(&positive);
    }
}

/// Updates raw gamepad state.
///
/// This function intentionally does NOT know anything
/// about emulator bindings.
pub fn update_gamepad_input(
    glfw: &glfw::Glfw,
    input: &mut InputState,
    joystick_id: lunaris_gui_common::input::enums::JoystickId,
) {
    let js = glfw.get_joystick(to_glfw_joystick_id(joystick_id));

    let Some(state) = js.get_gamepad_state() else {
        return;
    };

    //
    // Buttons
    //

    update_gamepad_button(input, &state, ButtonA);

    update_gamepad_button(input, &state, ButtonB);

    update_gamepad_button(input, &state, ButtonX);

    update_gamepad_button(input, &state, ButtonY);

    update_gamepad_button(input, &state, ButtonDpadUp);

    update_gamepad_button(input, &state, ButtonDpadDown);

    update_gamepad_button(input, &state, ButtonDpadLeft);

    update_gamepad_button(input, &state, ButtonDpadRight);

    update_gamepad_button(input, &state, ButtonLeftBumper);

    update_gamepad_button(input, &state, ButtonRightBumper);

    update_gamepad_button(input, &state, ButtonStart);

    update_gamepad_button(input, &state, ButtonBack);

    //
    // Axes
    //
    update_gamepad_axis(input, &state, glfw::GamepadAxis::AxisLeftX);
    update_gamepad_axis(input, &state, glfw::GamepadAxis::AxisLeftY);

    update_gamepad_axis(input, &state, glfw::GamepadAxis::AxisRightX);
    update_gamepad_axis(input, &state, glfw::GamepadAxis::AxisRightY);

    update_gamepad_axis(input, &state, glfw::GamepadAxis::AxisLeftTrigger);
    update_gamepad_axis(input, &state, glfw::GamepadAxis::AxisRightTrigger);
}

fn to_glfw_joystick_id(
    joystick_id: lunaris_gui_common::input::enums::JoystickId,
) -> glfw::JoystickId {
    use lunaris_gui_common::input::enums::JoystickId::*;

    match joystick_id {
        Joystick1 => glfw::JoystickId::Joystick1,
        Joystick2 => glfw::JoystickId::Joystick2,
        Joystick3 => glfw::JoystickId::Joystick3,
        Joystick4 => glfw::JoystickId::Joystick4,
        Joystick5 => glfw::JoystickId::Joystick5,
        Joystick6 => glfw::JoystickId::Joystick6,
        Joystick7 => glfw::JoystickId::Joystick7,
        Joystick8 => glfw::JoystickId::Joystick8,
        Joystick9 => glfw::JoystickId::Joystick9,
        Joystick10 => glfw::JoystickId::Joystick10,
        Joystick11 => glfw::JoystickId::Joystick11,
        Joystick12 => glfw::JoystickId::Joystick12,
        Joystick13 => glfw::JoystickId::Joystick13,
        Joystick14 => glfw::JoystickId::Joystick14,
        Joystick15 => glfw::JoystickId::Joystick15,
        Joystick16 => glfw::JoystickId::Joystick16,
    }
}

fn glfw_to_config_keyboard_key(key: glfw::Key) -> lunaris_gui_common::input::enums::KeyboardKey {
    use lunaris_gui_common::input::enums::KeyboardKey as Key;

    match key {
        glfw::Key::Space => Key::Space,
        glfw::Key::Apostrophe => Key::Apostrophe,
        glfw::Key::Comma => Key::Comma,
        glfw::Key::Minus => Key::Minus,
        glfw::Key::Period => Key::Period,
        glfw::Key::Slash => Key::Slash,
        glfw::Key::Num0 => Key::Num0,
        glfw::Key::Num1 => Key::Num1,
        glfw::Key::Num2 => Key::Num2,
        glfw::Key::Num3 => Key::Num3,
        glfw::Key::Num4 => Key::Num4,
        glfw::Key::Num5 => Key::Num5,
        glfw::Key::Num6 => Key::Num6,
        glfw::Key::Num7 => Key::Num7,
        glfw::Key::Num8 => Key::Num8,
        glfw::Key::Num9 => Key::Num9,
        glfw::Key::Semicolon => Key::Semicolon,
        glfw::Key::Equal => Key::Equal,
        glfw::Key::A => Key::A,
        glfw::Key::B => Key::B,
        glfw::Key::C => Key::C,
        glfw::Key::D => Key::D,
        glfw::Key::E => Key::E,
        glfw::Key::F => Key::F,
        glfw::Key::G => Key::G,
        glfw::Key::H => Key::H,
        glfw::Key::I => Key::I,
        glfw::Key::J => Key::J,
        glfw::Key::K => Key::K,
        glfw::Key::L => Key::L,
        glfw::Key::M => Key::M,
        glfw::Key::N => Key::N,
        glfw::Key::O => Key::O,
        glfw::Key::P => Key::P,
        glfw::Key::Q => Key::Q,
        glfw::Key::R => Key::R,
        glfw::Key::S => Key::S,
        glfw::Key::T => Key::T,
        glfw::Key::U => Key::U,
        glfw::Key::V => Key::V,
        glfw::Key::W => Key::W,
        glfw::Key::X => Key::X,
        glfw::Key::Y => Key::Y,
        glfw::Key::Z => Key::Z,
        glfw::Key::LeftBracket => Key::LeftBracket,
        glfw::Key::Backslash => Key::Backslash,
        glfw::Key::RightBracket => Key::RightBracket,
        glfw::Key::GraveAccent => Key::GraveAccent,
        glfw::Key::World1 => Key::World1,
        glfw::Key::World2 => Key::World2,
        glfw::Key::Escape => Key::Escape,
        glfw::Key::Enter => Key::Enter,
        glfw::Key::Tab => Key::Tab,
        glfw::Key::Backspace => Key::Backspace,
        glfw::Key::Insert => Key::Insert,
        glfw::Key::Delete => Key::Delete,
        glfw::Key::Right => Key::Right,
        glfw::Key::Left => Key::Left,
        glfw::Key::Down => Key::Down,
        glfw::Key::Up => Key::Up,
        glfw::Key::PageUp => Key::PageUp,
        glfw::Key::PageDown => Key::PageDown,
        glfw::Key::Home => Key::Home,
        glfw::Key::End => Key::End,
        glfw::Key::CapsLock => Key::CapsLock,
        glfw::Key::ScrollLock => Key::ScrollLock,
        glfw::Key::NumLock => Key::NumLock,
        glfw::Key::PrintScreen => Key::PrintScreen,
        glfw::Key::Pause => Key::Pause,
        glfw::Key::F1 => Key::F1,
        glfw::Key::F2 => Key::F2,
        glfw::Key::F3 => Key::F3,
        glfw::Key::F4 => Key::F4,
        glfw::Key::F5 => Key::F5,
        glfw::Key::F6 => Key::F6,
        glfw::Key::F7 => Key::F7,
        glfw::Key::F8 => Key::F8,
        glfw::Key::F9 => Key::F9,
        glfw::Key::F10 => Key::F10,
        glfw::Key::F11 => Key::F11,
        glfw::Key::F12 => Key::F12,
        glfw::Key::F13 => Key::F13,
        glfw::Key::F14 => Key::F14,
        glfw::Key::F15 => Key::F15,
        glfw::Key::F16 => Key::F16,
        glfw::Key::F17 => Key::F17,
        glfw::Key::F18 => Key::F18,
        glfw::Key::F19 => Key::F19,
        glfw::Key::F20 => Key::F20,
        glfw::Key::F21 => Key::F21,
        glfw::Key::F22 => Key::F22,
        glfw::Key::F23 => Key::F23,
        glfw::Key::F24 => Key::F24,
        glfw::Key::F25 => Key::F25,
        glfw::Key::Kp0 => Key::Kp0,
        glfw::Key::Kp1 => Key::Kp1,
        glfw::Key::Kp2 => Key::Kp2,
        glfw::Key::Kp3 => Key::Kp3,
        glfw::Key::Kp4 => Key::Kp4,
        glfw::Key::Kp5 => Key::Kp5,
        glfw::Key::Kp6 => Key::Kp6,
        glfw::Key::Kp7 => Key::Kp7,
        glfw::Key::Kp8 => Key::Kp8,
        glfw::Key::Kp9 => Key::Kp9,
        glfw::Key::KpDecimal => Key::KpDecimal,
        glfw::Key::KpDivide => Key::KpDivide,
        glfw::Key::KpMultiply => Key::KpMultiply,
        glfw::Key::KpSubtract => Key::KpSubtract,
        glfw::Key::KpAdd => Key::KpAdd,
        glfw::Key::KpEnter => Key::KpEnter,
        glfw::Key::KpEqual => Key::KpEqual,
        glfw::Key::LeftShift => Key::LeftShift,
        glfw::Key::LeftControl => Key::LeftControl,
        glfw::Key::LeftAlt => Key::LeftAlt,
        glfw::Key::LeftSuper => Key::LeftSuper,
        glfw::Key::RightShift => Key::RightShift,
        glfw::Key::RightControl => Key::RightControl,
        glfw::Key::RightAlt => Key::RightAlt,
        glfw::Key::RightSuper => Key::RightSuper,
        glfw::Key::Menu => Key::Menu,
        glfw::Key::Unknown => Key::Unknown,
    }
}

fn glfw_to_config_gamepad_button(button: glfw::GamepadButton) -> GamepadButton {
    use lunaris_gui_common::input::enums::GamepadButton::*;

    match button {
        glfw::GamepadButton::ButtonA => ButtonA,
        glfw::GamepadButton::ButtonB => ButtonB,
        glfw::GamepadButton::ButtonX => ButtonX,
        glfw::GamepadButton::ButtonY => ButtonY,
        glfw::GamepadButton::ButtonLeftBumper => ButtonLeftBumper,
        glfw::GamepadButton::ButtonRightBumper => ButtonRightBumper,
        glfw::GamepadButton::ButtonBack => ButtonBack,
        glfw::GamepadButton::ButtonStart => ButtonStart,
        glfw::GamepadButton::ButtonGuide => ButtonGuide,
        glfw::GamepadButton::ButtonLeftThumb => ButtonLeftThumb,
        glfw::GamepadButton::ButtonRightThumb => ButtonRightThumb,
        glfw::GamepadButton::ButtonDpadUp => ButtonDpadUp,
        glfw::GamepadButton::ButtonDpadRight => ButtonDpadRight,
        glfw::GamepadButton::ButtonDpadDown => ButtonDpadDown,
        glfw::GamepadButton::ButtonDpadLeft => ButtonDpadLeft,
    }
}

fn glfw_to_config_axis(axis: glfw::GamepadAxis) -> GamepadAxis {
    use lunaris_gui_common::input::enums::GamepadAxis::*;

    match axis {
        glfw::GamepadAxis::AxisLeftX => AxisLeftX,
        glfw::GamepadAxis::AxisLeftY => AxisLeftY,
        glfw::GamepadAxis::AxisRightX => AxisRightX,
        glfw::GamepadAxis::AxisRightY => AxisRightY,
        glfw::GamepadAxis::AxisLeftTrigger => AxisLeftTrigger,
        glfw::GamepadAxis::AxisRightTrigger => AxisRightTrigger,
    }
}
