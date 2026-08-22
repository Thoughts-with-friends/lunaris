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
//!
//! # Latency
//!
//! A pad is read once per repaint, which is as often as the console is
//! advanced -- but reading *state* alone is not enough at that rate. A press
//! that begins and ends between two repaints leaves no trace in the state, so
//! it never reaches the cart at all: at 60 Hz that is any tap shorter than
//! ~17 ms, which is exactly the sort of input a rhythm game or a menu wants.
//!
//! So [`Pads::poll`] latches the presses out of the event queue it is draining
//! anyway, and merges them with the held state. A press is then seen on the
//! very first frame after it happened whatever its length, and a button that
//! was down for one microsecond still counts for one emulated frame.

use melonds::keys;

use crate::bindings::Bindings;

/// The pad button that steps the emulation speed: the left stick's click.
///
/// Not part of [`crate::bindings`] and not a DS button, so it is free unless
/// the user has deliberately bound it -- [`Pads::poll`] checks, and leaves it
/// alone when they have.
pub const SPEED_BUTTON: gilrs::Button = gilrs::Button::LeftThumb;

/// One repaint's worth of controller input.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct PadSample {
    /// The DS key mask every connected pad is holding between them, plus
    /// anything pressed and released since the last poll (active-high, see
    /// [`melonds::keys`]).
    pub keys: u32,
    /// How many times [`SPEED_BUTTON`] was clicked since the last poll.
    ///
    /// A count rather than a flag, and taken from the event queue rather than
    /// from the button's state: two clicks inside one repaint are two steps,
    /// and a button *held* is one step rather than sixty a second.
    pub speed_clicks: u32,
}

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

    /// Read every connected pad, returning what they are holding and how
    /// often the speed button was clicked.
    ///
    /// The queue is drained *for* its payload rather than merely to keep gilrs
    /// updating: a `ButtonPressed` is the only record of a press too short to
    /// still be held when this runs, and dropping those is what made a quick
    /// tap feel like a missed one. See the module's Latency note.
    pub fn poll(&mut self, bindings: &Bindings) -> PadSample {
        let Some(gilrs) = &mut self.gilrs else {
            return PadSample::default();
        };

        // Whether the speed button is doing its own job or the user's. Read
        // before the drain, since a binding wins over the built-in use.
        let speed_button_free = !bindings.is_bound(SPEED_BUTTON);
        let mut latched = 0;
        let mut speed_clicks = 0;
        while let Some(event) = gilrs.next_event() {
            let gilrs::EventType::ButtonPressed(button, _) = event.event else {
                continue;
            };
            latched |= bindings.button_bit(button);
            if speed_button_free && button == SPEED_BUTTON {
                speed_clicks += 1;
            }
        }

        self.connected.clear();
        let mut mask = latched;
        for (_id, pad) in gilrs.gamepads() {
            self.connected.push(pad.name().to_owned());
            mask |= bindings.pad_mask(&pad) | Self::stick_mask(&pad);
        }
        PadSample { keys: mask, speed_clicks }
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
