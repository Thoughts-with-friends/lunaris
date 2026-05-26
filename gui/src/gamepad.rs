use glfw::GamepadButton::*;
use nds_core::nds::{self, NDS};

pub fn handle_gamepad_input(io: &imgui::Io, glfw: &glfw::Glfw, nds: &mut NDS) {
    if !io.want_capture_keyboard {
        let js = glfw.get_joystick(glfw::JoystickId::Joystick1);

        fn handle_gamepad_button(
            nds: &mut NDS,
            state: &glfw::GamepadState,
            button: glfw::GamepadButton,
            nds_key: nds::Key,
        ) {
            match state.get_button_state(button) {
                glfw::Action::Press => nds.press_key(nds_key),
                glfw::Action::Release => nds.release_key(nds_key),
                _ => {}
            }
        }

        if js.is_gamepad() {
            if let Some(state) = js.get_gamepad_state() {
                handle_gamepad_button(nds, &state, ButtonB, nds::Key::A);
                handle_gamepad_button(nds, &state, ButtonA, nds::Key::B);
                handle_gamepad_button(nds, &state, ButtonY, nds::Key::X);
                handle_gamepad_button(nds, &state, ButtonX, nds::Key::Y);

                handle_gamepad_button(nds, &state, ButtonLeftBumper, nds::Key::L);
                handle_gamepad_button(nds, &state, ButtonRightBumper, nds::Key::R);
                handle_gamepad_button(nds, &state, ButtonLeftThumb, nds::Key::L);
                handle_gamepad_button(nds, &state, ButtonRightThumb, nds::Key::R);
                handle_gamepad_button(nds, &state, ButtonStart, nds::Key::Start);
                handle_gamepad_button(nds, &state, ButtonBack, nds::Key::Select);

                handle_gamepad_button(nds, &state, ButtonDpadUp, nds::Key::Up);
                handle_gamepad_button(nds, &state, ButtonDpadDown, nds::Key::Down);
                handle_gamepad_button(nds, &state, ButtonDpadLeft, nds::Key::Left);
                handle_gamepad_button(nds, &state, ButtonDpadRight, nds::Key::Right);
            }
        }
    }
}

pub fn handle_gamepad_axis(
    nds: &mut NDS,
    glfw: &glfw::Glfw,
    axis: glfw::GamepadAxis,
    negative_key: nds::Key,
    positive_key: nds::Key,
) {
    let js = glfw.get_joystick(glfw::JoystickId::Joystick1);
    let Some(state) = js.get_gamepad_state() else {
        return;
    };
    let value = state.get_axis(axis);

    const DEADZONE: f32 = 0.3;

    if value < -DEADZONE {
        nds.press_key(negative_key);
    } else {
        nds.release_key(negative_key);
    }

    if value > DEADZONE {
        nds.press_key(positive_key);
    } else {
        nds.release_key(positive_key);
    }
}
