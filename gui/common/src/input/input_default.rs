use crate::input::enums::{
    AxisDirection, BindKey, GamepadAxis::*, GamepadButton::*, InputBinding, InputSource,
    KeyboardKey::*,
};

pub fn default_input_bindings() -> Vec<InputBinding> {
    vec![
        //
        // Keyboard
        //
        InputBinding { sources: vec![InputSource::Keyboard { key: Up }], target: BindKey::Up },
        InputBinding { sources: vec![InputSource::Keyboard { key: Down }], target: BindKey::Down },
        InputBinding { sources: vec![InputSource::Keyboard { key: Left }], target: BindKey::Left },
        InputBinding {
            sources: vec![InputSource::Keyboard { key: Right }],
            target: BindKey::Right,
        },
        InputBinding { sources: vec![InputSource::Keyboard { key: A }], target: BindKey::A },
        InputBinding { sources: vec![InputSource::Keyboard { key: B }], target: BindKey::B },
        InputBinding { sources: vec![InputSource::Keyboard { key: X }], target: BindKey::X },
        InputBinding { sources: vec![InputSource::Keyboard { key: Y }], target: BindKey::Y },
        InputBinding { sources: vec![InputSource::Keyboard { key: L }], target: BindKey::L },
        InputBinding { sources: vec![InputSource::Keyboard { key: R }], target: BindKey::R },
        InputBinding { sources: vec![InputSource::Keyboard { key: T }], target: BindKey::Start },
        InputBinding { sources: vec![InputSource::Keyboard { key: E }], target: BindKey::Select },
        //
        // Gamepad buttons (no joystick id anymore)
        //
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonDpadUp }],
            target: BindKey::Up,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonDpadDown }],
            target: BindKey::Down,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonDpadLeft }],
            target: BindKey::Left,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonDpadRight }],
            target: BindKey::Right,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonB }],
            target: BindKey::A,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonA }],
            target: BindKey::B,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonY }],
            target: BindKey::X,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonX }],
            target: BindKey::Y,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonLeftBumper }],
            target: BindKey::L,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonRightBumper }],
            target: BindKey::R,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonStart }],
            target: BindKey::Start,
        },
        InputBinding {
            sources: vec![InputSource::GamepadButton { button: ButtonBack }],
            target: BindKey::Select,
        },
        //
        // Left stick
        //
        InputBinding {
            sources: vec![InputSource::GamepadAxis {
                axis: AxisLeftX,
                direction: AxisDirection::Negative,
            }],
            target: BindKey::Left,
        },
        InputBinding {
            sources: vec![InputSource::GamepadAxis {
                axis: AxisLeftX,
                direction: AxisDirection::Positive,
            }],
            target: BindKey::Right,
        },
        InputBinding {
            sources: vec![InputSource::GamepadAxis {
                axis: AxisLeftY,
                direction: AxisDirection::Negative,
            }],
            target: BindKey::Up,
        },
        InputBinding {
            sources: vec![InputSource::GamepadAxis {
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
                axis: AxisLeftTrigger,
                direction: AxisDirection::Positive,
            }],
            target: BindKey::L,
        },
        InputBinding {
            sources: vec![InputSource::GamepadAxis {
                axis: AxisRightTrigger,
                direction: AxisDirection::Positive,
            }],
            target: BindKey::R,
        },
    ]
}
