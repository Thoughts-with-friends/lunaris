//! Game controller input.
//!
//! egui reports keyboard and pointer only, so pads are read straight from the
//! host through [`gilrs`] — XInput on Windows, evdev on Linux, IOKit on macOS,
//! all behind one API, with SDL's controller-mapping database vendored so that
//! a pad's buttons land where their labels say.
//!
//! # What it maps to
//!
//! Whatever [`crate::bindings`] says, which starts as melonDS's own map and is
//! editable in `Config ▸ Input and hotkeys`. The one thing not in that map is
//! the left stick: it stands in for the D-pad unconditionally, because a cart
//! cannot tell the two apart and nobody wants to bind a stick to four
//! directions by hand.
//!
//! Input is *merged* with the keyboard rather than replacing it: a key and a
//! button held together are one press, and neither has to be chosen up front.

use melonds::keys;

use crate::bindings::Bindings;

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
            log::warn!("no controller support ({e}); keyboard only");
        }
        Self { gilrs, connected: Vec::new() }
    }

    /// Drain the event queue and return the DS key mask every connected pad is
    /// holding between them (active-high, see [`melonds::keys`]).
    ///
    /// The events are drained rather than acted on: gilrs only updates the
    /// state this reads while its queue is being pumped, and reading state is
    /// what makes a held button hold rather than repeat.
    pub fn poll(&mut self, bindings: &Bindings) -> u32 {
        let Some(gilrs) = &mut self.gilrs else {
            return 0;
        };
        while gilrs.next_event().is_some() {}

        self.connected.clear();
        let mut mask = 0;
        for (_id, pad) in gilrs.gamepads() {
            self.connected.push(pad.name().to_owned());
            mask |= bindings.pad_mask(&pad) | Self::stick_mask(&pad);
        }
        mask
    }

    /// The first button any connected pad is holding, for the Input dialog to
    /// bind.
    ///
    /// Read from state rather than from an event, so that the dialog cannot
    /// miss a press that landed between two repaints — the same reason
    /// [`Self::poll`] does.
    #[must_use]
    pub fn first_pressed(&mut self) -> Option<gilrs::Button> {
        let gilrs = self.gilrs.as_mut()?;
        while gilrs.next_event().is_some() {}
        gilrs.gamepads().find_map(|(_id, pad)| {
            crate::bindings::PAD_BUTTONS
                .iter()
                .find(|(button, _)| pad.is_pressed(*button))
                .map(|(button, _)| *button)
        })
    }

    /// The names of the pads the last [`Self::poll`] saw.
    pub fn connected(&self) -> &[String] {
        &self.connected
    }

    /// The left stick as a DS D-pad mask.
    ///
    /// Not part of [`crate::bindings`] on purpose: a stick is an axis and the
    /// D-pad is four switches, so there is nothing sensible to *bind* it to
    /// one of. Many pads also report their D-pad as this axis, which is the
    /// other reason it is always read.
    fn stick_mask(pad: &gilrs::Gamepad<'_>) -> u32 {
        use gilrs::Axis;

        let mut mask = 0;
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
