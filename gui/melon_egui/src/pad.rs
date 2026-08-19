//! Game controller input.
//!
//! egui reports keyboard and pointer only, so pads are read straight from the
//! host through [`gilrs`] — XInput on Windows, evdev on Linux, IOKit on macOS,
//! all behind one API, with SDL's controller-mapping database vendored so that
//! a pad's buttons land where their labels say.
//!
//! # What it maps to
//!
//! The face buttons follow the DS's own layout rather than the pad's labels:
//! the DS's A is on the right and its B at the bottom, which is the same
//! arrangement a modern pad has, so A/B/X/Y map to East/South/North/West by
//! position. Everything else is the obvious one — shoulders to L and R, Start
//! and Select to Start and Select, and both the D-pad and the left stick to the
//! D-pad, since a cart cannot tell them apart.
//!
//! Input is *merged* with the keyboard rather than replacing it: a key and a
//! button held together are one press, and neither has to be chosen up front.

use melonds::keys;

/// Past this, the left stick counts as a direction held. Generous, because the
/// D-pad it stands in for is a switch and not an axis — an eighth of a push
/// registering as "left" would make menus unusable.
const STICK_THRESHOLD: f32 = 0.5;

/// The host's game controllers, polled once per repaint.
pub struct Pads {
    /// `None` when the platform has no controller support at all, which is not
    /// an error worth stopping for — it just means no pad can be used.
    gilrs: Option<gilrs::Gilrs>,
    /// What the last poll produced, so the front end can show it without
    /// polling again.
    connected: Vec<String>,
}

impl Pads {
    /// Open the controller subsystem. Failure here (no permission, no back end)
    /// leaves a [`Pads`] that reports no pads rather than none at all.
    pub fn new() -> Self {
        let (gilrs, error) = match gilrs::Gilrs::new() {
            Ok(gilrs) => (Some(gilrs), None),
            Err(e) => (None, Some(e.to_string())),
        };
        if let Some(e) = error {
            eprintln!("melon_egui: no controller support ({e}); keyboard only");
        }
        Self { gilrs, connected: Vec::new() }
    }

    /// Drain the event queue and return the DS key mask every connected pad is
    /// holding between them (active-high, see [`melonds::keys`]).
    ///
    /// The events are drained rather than acted on: gilrs only updates the
    /// state this reads while its queue is being pumped, and reading state is
    /// what makes a held button hold rather than repeat.
    pub fn poll(&mut self) -> u32 {
        let Some(gilrs) = &mut self.gilrs else {
            return 0;
        };
        while gilrs.next_event().is_some() {}

        self.connected.clear();
        let mut mask = 0;
        for (_id, pad) in gilrs.gamepads() {
            self.connected.push(pad.name().to_owned());
            mask |= Self::mask_of(&pad);
        }
        mask
    }

    /// The names of the pads the last [`Self::poll`] saw.
    pub fn connected(&self) -> &[String] {
        &self.connected
    }

    /// One pad's buttons and stick as a DS key mask.
    fn mask_of(pad: &gilrs::Gamepad<'_>) -> u32 {
        use gilrs::{Axis, Button};

        const BUTTONS: [(Button, u32); 10] = [
            // By position, not by label: the DS's A is the right-hand face
            // button and its B the bottom one.
            (Button::East, keys::A),
            (Button::South, keys::B),
            (Button::North, keys::X),
            (Button::West, keys::Y),
            (Button::LeftTrigger, keys::L),
            (Button::RightTrigger, keys::R),
            (Button::Start, keys::START),
            (Button::Select, keys::SELECT),
            (Button::DPadUp, keys::UP),
            (Button::DPadDown, keys::DOWN),
        ];

        let mut mask = 0;
        for (button, bit) in BUTTONS {
            if pad.is_pressed(button) {
                mask |= bit;
            }
        }
        // Split out only because the array above is already at the length that
        // reads well; there is nothing different about these two.
        for (button, bit) in [(Button::DPadLeft, keys::LEFT), (Button::DPadRight, keys::RIGHT)] {
            if pad.is_pressed(button) {
                mask |= bit;
            }
        }

        // A pad whose D-pad is reported as an axis (many are) shows up here
        // instead, and so does the left stick. Up is positive on both.
        let x = pad.value(Axis::LeftStickX);
        let y = pad.value(Axis::LeftStickY);
        if x <= -STICK_THRESHOLD {
            mask |= keys::LEFT;
        }
        if x >= STICK_THRESHOLD {
            mask |= keys::RIGHT;
        }
        if y >= STICK_THRESHOLD {
            mask |= keys::UP;
        }
        if y <= -STICK_THRESHOLD {
            mask |= keys::DOWN;
        }
        mask
    }
}
