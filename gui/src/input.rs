use std::collections::HashSet;

use glfw::GamepadButton::*;
use nds_core::nds::{self, NDS};

use crate::config::{AxisDirection, BindKey, InputBinding, InputSource};

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
    pub keyboard: HashSet<glfw::Key>,

    /// Currently pressed gamepad buttons.
    pub gamepad_buttons: HashSet<(glfw::JoystickId, glfw::GamepadButton)>,

    /// Currently active gamepad axis directions.
    pub gamepad_axes: HashSet<(glfw::JoystickId, glfw::GamepadAxis, AxisDirection)>,
}

impl InputState {
    pub fn is_source_active(&self, source: &InputSource) -> bool {
        match source {
            InputSource::Keyboard { key } => self.keyboard.contains(key),

            InputSource::GamepadButton { joystick, button } => {
                self.gamepad_buttons.contains(&(*joystick, *button))
            }

            InputSource::GamepadAxis {
                joystick,
                axis,
                direction,
            } => self.gamepad_axes.contains(&(*joystick, *axis, *direction)),
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
        let active = binding
            .sources
            .iter()
            .all(|source| input.is_source_active(source));

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

    apply_key(
        nds,
        active_keys.contains(&BindKey::Select),
        nds::Key::Select,
    );
}

/// Updates raw keyboard state.
///
/// This function intentionally does NOT know anything
/// about emulator bindings.
pub fn update_keyboard_input(input: &mut InputState, key: glfw::Key, pressed: bool) {
    if pressed {
        input.keyboard.insert(key);
    } else {
        input.keyboard.remove(&key);
    }
}

fn update_gamepad_button(
    input: &mut InputState,
    joystick: glfw::JoystickId,
    state: &glfw::GamepadState,
    button: glfw::GamepadButton,
) {
    let key = (joystick, button);

    if matches!(state.get_button_state(button), glfw::Action::Press) {
        input.gamepad_buttons.insert(key);
    } else {
        input.gamepad_buttons.remove(&key);
    }
}

fn update_gamepad_axis(
    input: &mut InputState,
    joystick: glfw::JoystickId,
    state: &glfw::GamepadState,
    axis: glfw::GamepadAxis,
) {
    let value = state.get_axis(axis);

    let negative = (joystick, axis, AxisDirection::Negative);

    let positive = (joystick, axis, AxisDirection::Positive);

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
pub fn update_gamepad_input(glfw: &glfw::Glfw, input: &mut InputState) {
    let joystick = glfw::JoystickId::Joystick1;

    let js = glfw.get_joystick(joystick);

    let Some(state) = js.get_gamepad_state() else {
        return;
    };

    //
    // Buttons
    //

    update_gamepad_button(input, joystick, &state, ButtonA);

    update_gamepad_button(input, joystick, &state, ButtonB);

    update_gamepad_button(input, joystick, &state, ButtonX);

    update_gamepad_button(input, joystick, &state, ButtonY);

    update_gamepad_button(input, joystick, &state, ButtonDpadUp);

    update_gamepad_button(input, joystick, &state, ButtonDpadDown);

    update_gamepad_button(input, joystick, &state, ButtonDpadLeft);

    update_gamepad_button(input, joystick, &state, ButtonDpadRight);

    update_gamepad_button(input, joystick, &state, ButtonLeftBumper);

    update_gamepad_button(input, joystick, &state, ButtonRightBumper);

    update_gamepad_button(input, joystick, &state, ButtonStart);

    update_gamepad_button(input, joystick, &state, ButtonBack);

    //
    // Axes
    //

    update_gamepad_axis(input, joystick, &state, glfw::GamepadAxis::AxisLeftX);

    update_gamepad_axis(input, joystick, &state, glfw::GamepadAxis::AxisLeftY);

    update_gamepad_axis(input, joystick, &state, glfw::GamepadAxis::AxisRightX);

    update_gamepad_axis(input, joystick, &state, glfw::GamepadAxis::AxisRightY);

    update_gamepad_axis(input, joystick, &state, glfw::GamepadAxis::AxisLeftTrigger);

    update_gamepad_axis(input, joystick, &state, glfw::GamepadAxis::AxisRightTrigger);
}
