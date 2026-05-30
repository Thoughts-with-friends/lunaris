use glfw::GamepadButton::*;
use nds_core::nds::{self, NDS};

const DEAD_ZONE: f32 = 0.3;

/// NOTE:
/// Input sources (keyboard, dpad, analog stick, triggers, etc.)
/// must NOT call `nds.press_key()` / `nds.release_key()` independently.
///
/// If multiple sources update the same NDS key in a single frame,
/// they can overwrite each other.
///
/// Example:
/// - DPad presses `Down`
/// - Analog stick is neutral
/// - Stick handler releases `Down` in the same frame
/// - Final result becomes released
///
/// To avoid this, all physical inputs are first merged into
/// `InputState`, then applied to the NDS exactly once per frame.
#[derive(Default)]
pub struct InputState {
    up: bool,
    down: bool,
    left: bool,
    right: bool,

    a: bool,
    b: bool,
    x: bool,
    y: bool,

    l: bool,
    r: bool,

    start: bool,
    select: bool,
}

fn apply_key(nds: &mut NDS, pressed: bool, key: nds::Key) {
    if pressed {
        nds.press_key(key);
    } else {
        nds.release_key(key);
    }
}

pub fn apply_input_state(nds: &mut NDS, state: &InputState) {
    apply_key(nds, state.up, nds::Key::Up);
    apply_key(nds, state.down, nds::Key::Down);
    apply_key(nds, state.left, nds::Key::Left);
    apply_key(nds, state.right, nds::Key::Right);

    apply_key(nds, state.a, nds::Key::A);
    apply_key(nds, state.b, nds::Key::B);
    apply_key(nds, state.x, nds::Key::X);
    apply_key(nds, state.y, nds::Key::Y);

    apply_key(nds, state.l, nds::Key::L);
    apply_key(nds, state.r, nds::Key::R);

    apply_key(nds, state.start, nds::Key::Start);
    apply_key(nds, state.select, nds::Key::Select);
}

fn button_pressed(state: &glfw::GamepadState, button: glfw::GamepadButton) -> bool {
    matches!(state.get_button_state(button), glfw::Action::Press)
}

pub fn update_keyboard_input(input: &mut InputState, key: glfw::Key, pressed: bool) {
    match key {
        glfw::Key::A => input.a = pressed,
        glfw::Key::B => input.b = pressed,
        glfw::Key::X => input.x = pressed,
        glfw::Key::Y => input.y = pressed,

        glfw::Key::Up => input.up = pressed,
        glfw::Key::Down => input.down = pressed,
        glfw::Key::Left => input.left = pressed,
        glfw::Key::Right => input.right = pressed,

        glfw::Key::L => input.l = pressed,
        glfw::Key::R => input.r = pressed,

        glfw::Key::T => input.start = pressed,
        glfw::Key::E => input.select = pressed,

        _ => {}
    }
}

pub fn update_gamepad_input(glfw: &glfw::Glfw, input: &mut InputState) {
    let js = glfw.get_joystick(glfw::JoystickId::Joystick1);

    let Some(state) = js.get_gamepad_state() else {
        return;
    };

    //
    // Left stick
    //

    let lx = state.get_axis(glfw::GamepadAxis::AxisLeftX);
    let ly = state.get_axis(glfw::GamepadAxis::AxisLeftY);

    input.left |= lx < -DEAD_ZONE;
    input.right |= lx > DEAD_ZONE;

    input.up |= ly < -DEAD_ZONE;
    input.down |= ly > DEAD_ZONE;

    //
    // DPad
    //

    input.up |= button_pressed(&state, ButtonDpadUp);
    input.down |= button_pressed(&state, ButtonDpadDown);

    input.left |= button_pressed(&state, ButtonDpadLeft);
    input.right |= button_pressed(&state, ButtonDpadRight);

    //
    // Face buttons
    //

    input.a |= button_pressed(&state, ButtonB);
    input.b |= button_pressed(&state, ButtonA);

    input.x |= button_pressed(&state, ButtonY);
    input.y |= button_pressed(&state, ButtonX);

    //
    // Bumpers
    //

    input.l |= button_pressed(&state, ButtonLeftBumper);
    input.r |= button_pressed(&state, ButtonRightBumper);

    //
    // Triggers
    //

    input.l |= state.get_axis(glfw::GamepadAxis::AxisLeftTrigger) > DEAD_ZONE;

    input.r |= state.get_axis(glfw::GamepadAxis::AxisRightTrigger) > DEAD_ZONE;

    //
    // System buttons
    //

    input.start |= button_pressed(&state, ButtonStart);
    input.select |= button_pressed(&state, ButtonBack);
}
