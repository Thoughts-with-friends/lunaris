//! Keyboard and gamepad input.

use core_base::{Key, Nds};
use egui::Key as EKey;
use gilrs::{Axis, Button, Gilrs};

/// Host key to DS button mapping.
const KEYS: [(EKey, Key); 12] = [
    (EKey::X, Key::A),
    (EKey::Z, Key::B),
    (EKey::S, Key::X),
    (EKey::A, Key::Y),
    (EKey::Q, Key::L),
    (EKey::W, Key::R),
    (EKey::Enter, Key::Start),
    (EKey::Backspace, Key::Select),
    (EKey::ArrowUp, Key::Up),
    (EKey::ArrowDown, Key::Down),
    (EKey::ArrowLeft, Key::Left),
    (EKey::ArrowRight, Key::Right),
];

/// Gamepad button to DS button mapping. The face buttons follow the DS
/// layout rather than the host's labels: `South` is B, `East` is A.
const PAD: [(Button, Key); 12] = [
    (Button::East, Key::A),
    (Button::South, Key::B),
    (Button::North, Key::X),
    (Button::West, Key::Y),
    (Button::LeftTrigger, Key::L),
    (Button::RightTrigger, Key::R),
    (Button::Start, Key::Start),
    (Button::Select, Key::Select),
    (Button::DPadUp, Key::Up),
    (Button::DPadDown, Key::Down),
    (Button::DPadLeft, Key::Left),
    (Button::DPadRight, Key::Right),
];

/// Deflection past which the left stick counts as a directional press.
const STICK_THRESHOLD: f32 = 0.5;

/// Polls every input device and pushes the combined state into the console.
pub struct Input {
    /// `None` when no gamepad backend could be initialised, which is not an
    /// error: the keyboard still works.
    gilrs: Option<Gilrs>,
}

impl Default for Input {
    fn default() -> Self {
        Self { gilrs: Gilrs::new().ok() }
    }
}

impl Input {
    /// Applies the current keyboard and gamepad state for this frame.
    ///
    /// A button counts as held if any device holds it, so the two input
    /// methods can be used interchangeably.
    pub fn apply(&mut self, ctx: &egui::Context, nds: &mut Nds) {
        let mut held = [false; 12];
        ctx.input(|state| {
            for (slot, (host, _)) in KEYS.iter().enumerate() {
                held[slot] = state.key_down(*host);
            }
        });
        if let Some(gilrs) = self.gilrs.as_mut() {
            // Draining the event queue is what refreshes each gamepad's
            // cached state; the state itself is read below.
            while gilrs.next_event().is_some() {}
            for (_, pad) in gilrs.gamepads() {
                for (slot, (button, _)) in PAD.iter().enumerate() {
                    held[slot] |= pad.is_pressed(*button);
                }
                let (x, y) = (pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY));
                held[8] |= y > STICK_THRESHOLD;
                held[9] |= y < -STICK_THRESHOLD;
                held[10] |= x < -STICK_THRESHOLD;
                held[11] |= x > STICK_THRESHOLD;
            }
        }
        for (slot, (_, key)) in KEYS.iter().enumerate() {
            nds.set_key(*key, held[slot]);
        }
    }

    /// Name of the first connected gamepad, for the status line.
    pub fn gamepad_name(&self) -> Option<String> {
        let gilrs = self.gilrs.as_ref()?;
        gilrs.gamepads().next().map(|(_, pad)| pad.name().to_owned())
    }
}
