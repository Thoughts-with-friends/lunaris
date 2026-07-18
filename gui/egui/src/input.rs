//! Keyboard + gamepad → NDS key resolution.
//!
//! Bindings are fixed for this first egui implementation (see the note in
//! `config.rs`); the merge strategy — collect every active physical source
//! into one set per frame, then resolve each NDS key exactly once — mirrors
//! `docs/design/egui-migration-design.md` §8.6 / the original
//! `gui/src/input.rs` doc comment, so that e.g. a D-pad release doesn't
//! clobber a simultaneously-held analog-stick direction.

use nds_core::nds::{Key as NdsKey, NDS};

/// `NdsKey` derives only `PartialEq` (not `Eq`/`Hash`), so active keys are
/// tracked as a small `Vec` instead of a `HashSet`. There are at most 12
/// bindable keys, so linear `contains` is effectively free here.
type KeySet = Vec<NdsKey>;

/// Default keyboard bindings, matching the imgui front end's defaults.
const KEYBOARD_BINDINGS: &[(egui::Key, NdsKey)] = &[
    (egui::Key::ArrowUp, NdsKey::Up),
    (egui::Key::ArrowDown, NdsKey::Down),
    (egui::Key::ArrowLeft, NdsKey::Left),
    (egui::Key::ArrowRight, NdsKey::Right),
    (egui::Key::A, NdsKey::A),
    (egui::Key::B, NdsKey::B),
    (egui::Key::X, NdsKey::X),
    (egui::Key::Y, NdsKey::Y),
    (egui::Key::L, NdsKey::L),
    (egui::Key::R, NdsKey::R),
    (egui::Key::T, NdsKey::Start),
    (egui::Key::E, NdsKey::Select),
];

/// Default gamepad button bindings.
///
/// gilrs names the shoulder bumpers `LeftTrigger`/`RightTrigger` (the analog
/// triggers are `LeftTrigger2`/`RightTrigger2`) — see
/// `docs/design/egui-migration-design.md` §8.3 for this and the East/South
/// face-button swap (Nintendo A/B are mirrored relative to Xbox-style pads).
const GAMEPAD_BUTTON_BINDINGS: &[(gilrs::Button, NdsKey)] = &[
    (gilrs::Button::East, NdsKey::A),
    (gilrs::Button::South, NdsKey::B),
    (gilrs::Button::North, NdsKey::X),
    (gilrs::Button::West, NdsKey::Y),
    (gilrs::Button::DPadUp, NdsKey::Up),
    (gilrs::Button::DPadDown, NdsKey::Down),
    (gilrs::Button::DPadLeft, NdsKey::Left),
    (gilrs::Button::DPadRight, NdsKey::Right),
    (gilrs::Button::LeftTrigger, NdsKey::L),
    (gilrs::Button::RightTrigger, NdsKey::R),
    (gilrs::Button::Start, NdsKey::Start),
    (gilrs::Button::Select, NdsKey::Select),
];

const STICK_DEAD_ZONE: f32 = 0.3;

/// Resolves keyboard state (from `egui::InputState`) into the set of
/// currently-desired NDS keys.
fn keyboard_keys(ctx: &egui::Context) -> KeySet {
    ctx.input(|i| {
        KEYBOARD_BINDINGS
            .iter()
            .filter(|(ekey, _)| i.key_down(*ekey))
            .map(|(_, ndskey)| *ndskey)
            .collect()
    })
}

/// Resolves the first connected gamepad's state into the set of
/// currently-desired NDS keys (buttons + left-stick-as-D-pad).
fn gamepad_keys(gilrs: &gilrs::Gilrs) -> KeySet {
    let mut keys = KeySet::new();
    let Some((_, gamepad)) = gilrs.gamepads().next() else {
        return keys;
    };

    for (button, ndskey) in GAMEPAD_BUTTON_BINDINGS {
        if gamepad.is_pressed(*button) {
            keys.push(*ndskey);
        }
    }

    let stick_x = gamepad.value(gilrs::Axis::LeftStickX);
    let stick_y = gamepad.value(gilrs::Axis::LeftStickY);
    if stick_x < -STICK_DEAD_ZONE {
        keys.push(NdsKey::Left);
    }
    if stick_x > STICK_DEAD_ZONE {
        keys.push(NdsKey::Right);
    }
    // gilrs reports +Y as up; invert so it matches D-pad "down" semantics.
    if stick_y < -STICK_DEAD_ZONE {
        keys.push(NdsKey::Down);
    }
    if stick_y > STICK_DEAD_ZONE {
        keys.push(NdsKey::Up);
    }

    keys
}

/// All bindable NDS keys, used to resolve press/release transitions.
const ALL_KEYS: &[NdsKey] = &[
    NdsKey::Up,
    NdsKey::Down,
    NdsKey::Left,
    NdsKey::Right,
    NdsKey::A,
    NdsKey::B,
    NdsKey::X,
    NdsKey::Y,
    NdsKey::L,
    NdsKey::R,
    NdsKey::Start,
    NdsKey::Select,
];

/// Drains pending gilrs events (keeps its internal gamepad cache fresh),
/// merges keyboard + gamepad state, and applies the result to `nds`.
pub fn apply_input(nds: &mut NDS, ctx: &egui::Context, gilrs: &mut gilrs::Gilrs) {
    while gilrs.next_event().is_some() {}

    let mut active = keyboard_keys(ctx);
    active.append(&mut gamepad_keys(gilrs));

    for key in ALL_KEYS {
        if active.contains(key) {
            nds.press_key(*key);
        } else {
            nds.release_key(*key);
        }
    }
}
