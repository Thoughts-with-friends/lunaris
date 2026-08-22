//! Which key and which pad button each DS button answers to.
//!
//! # Why this is a setting rather than a constant
//!
//! The defaults are melonDS's own, and most people never touch them. But a
//! keyboard that is not US QWERTY puts `Z` and `X` somewhere else entirely, and
//! a pad whose face buttons are labelled the other way round needs the two
//! swapped — neither of which a fixed table can answer. So the whole map is
//! saved with the rest of the instance's settings and edited in
//! `Config ▸ Input and hotkeys`, laid out the way melonDS lays it out.
//!
//! # How it is stored
//!
//! By **name**, not by number: `"X"` and `"East"` rather than the discriminants
//! `egui::Key` and `gilrs::Button` happen to have today. A settings file
//! written by one version has to keep meaning the same thing after either
//! library renumbers its enum, and a person editing the JSON by hand should be
//! able to read it.
//!
//! An unknown name is dropped rather than refused, leaving that button
//! unbound — a file from a newer version must not stop the emulator starting.

use melonds::keys;

/// One of the DS's twelve buttons.
///
/// The touchscreen is deliberately absent: it is not a button, and it is
/// already bound to clicking the bottom screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DsInput {
    A,
    B,
    X,
    Y,
    L,
    R,
    Start,
    Select,
    Up,
    Down,
    Left,
    Right,
}

impl DsInput {
    /// In the order melonDS's Input dialog lists them.
    pub const ALL: [Self; 12] = [
        Self::A,
        Self::B,
        Self::X,
        Self::Y,
        Self::L,
        Self::R,
        Self::Start,
        Self::Select,
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
    ];

    /// The bit this button sets in the core's key mask.
    #[must_use]
    pub const fn bit(self) -> u32 {
        match self {
            Self::A => keys::A,
            Self::B => keys::B,
            Self::X => keys::X,
            Self::Y => keys::Y,
            Self::L => keys::L,
            Self::R => keys::R,
            Self::Start => keys::START,
            Self::Select => keys::SELECT,
            Self::Up => keys::UP,
            Self::Down => keys::DOWN,
            Self::Left => keys::LEFT,
            Self::Right => keys::RIGHT,
        }
    }

    /// What the dialog calls it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::X => "X",
            Self::Y => "Y",
            Self::L => "L",
            Self::R => "R",
            Self::Start => "Start",
            Self::Select => "Select",
            Self::Up => "Up",
            Self::Down => "Down",
            Self::Left => "Left",
            Self::Right => "Right",
        }
    }

    /// melonDS's default key for this button.
    const fn default_key(self) -> egui::Key {
        match self {
            Self::A => egui::Key::X,
            Self::B => egui::Key::Z,
            Self::X => egui::Key::S,
            Self::Y => egui::Key::A,
            Self::L => egui::Key::Q,
            Self::R => egui::Key::W,
            Self::Start => egui::Key::Enter,
            Self::Select => egui::Key::Backspace,
            Self::Up => egui::Key::ArrowUp,
            Self::Down => egui::Key::ArrowDown,
            Self::Left => egui::Key::ArrowLeft,
            Self::Right => egui::Key::ArrowRight,
        }
    }

    /// The default pad button.
    ///
    /// The face buttons go by **position**, not by label: the DS's A is its
    /// right-hand face button and its B the bottom one, which is where a modern
    /// pad's East and South sit. Following the letters instead would put A and
    /// B where a player's thumb does not expect them.
    const fn default_button(self) -> gilrs::Button {
        match self {
            Self::A => gilrs::Button::East,
            Self::B => gilrs::Button::South,
            Self::X => gilrs::Button::North,
            Self::Y => gilrs::Button::West,
            Self::L => gilrs::Button::LeftTrigger,
            Self::R => gilrs::Button::RightTrigger,
            Self::Start => gilrs::Button::Start,
            Self::Select => gilrs::Button::Select,
            Self::Up => gilrs::Button::DPadUp,
            Self::Down => gilrs::Button::DPadDown,
            Self::Left => gilrs::Button::DPadLeft,
            Self::Right => gilrs::Button::DPadRight,
        }
    }
}

/// What one DS button is bound to. Either half may be unbound.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Binding {
    /// An `egui::Key` by its own name, as `Key::name` spells it.
    pub key: Option<String>,
    /// A `gilrs::Button` by the name in [`PAD_BUTTONS`].
    pub button: Option<String>,
}

impl Binding {
    /// The key this is bound to, if the name still means something.
    #[must_use]
    pub fn key(&self) -> Option<egui::Key> {
        self.key.as_deref().and_then(egui::Key::from_name)
    }

    /// The pad button this is bound to, if the name still means something.
    #[must_use]
    pub fn button(&self) -> Option<gilrs::Button> {
        self.button.as_deref().and_then(button_from_name)
    }
}

/// The whole map, one entry per DS button.
///
/// A `Vec` of pairs rather than a map keyed by [`DsInput`]: it keeps the JSON
/// in the dialog's order, which is what makes a hand-edited file readable.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Bindings(Vec<(DsInput, Binding)>);

impl Default for Bindings {
    /// melonDS's defaults.
    fn default() -> Self {
        Self(
            DsInput::ALL
                .into_iter()
                .map(|input| {
                    (
                        input,
                        Binding {
                            key: Some(input.default_key().name().to_owned()),
                            button: Some(button_name(input.default_button()).to_owned()),
                        },
                    )
                })
                .collect(),
        )
    }
}

impl Bindings {
    /// Every DS button and what it is bound to, in the dialog's order.
    ///
    /// A button missing from the file reads as unbound rather than as an error,
    /// and one named twice keeps the first — so a hand-edited file is always
    /// usable, however odd.
    pub fn entries(&self) -> impl Iterator<Item = (DsInput, Binding)> {
        DsInput::ALL.into_iter().map(|input| (input, self.get(input)))
    }

    /// What `input` is bound to, or nothing.
    #[must_use]
    pub fn get(&self, input: DsInput) -> Binding {
        self.0.iter().find(|(it, _)| *it == input).map(|(_, b)| b.clone()).unwrap_or_default()
    }

    /// Bind `input`, replacing whatever it had.
    pub fn set(&mut self, input: DsInput, binding: Binding) {
        match self.0.iter_mut().find(|(it, _)| *it == input) {
            Some(slot) => slot.1 = binding,
            None => self.0.push((input, binding)),
        }
    }

    /// Bind `input`'s key, taking it off whatever else had it.
    ///
    /// Exclusive because two DS buttons on one key is never what somebody
    /// meant: the second one silently shadows the first, and the dialog would
    /// show a map that does not behave the way it reads.
    pub fn bind_key(&mut self, input: DsInput, key: egui::Key) {
        for (other, binding) in &mut self.0 {
            if *other != input && binding.key() == Some(key) {
                binding.key = None;
            }
        }
        let mut binding = self.get(input);
        binding.key = Some(key.name().to_owned());
        self.set(input, binding);
    }

    /// Bind `input`'s pad button, taking it off whatever else had it.
    pub fn bind_button(&mut self, input: DsInput, button: gilrs::Button) {
        for (other, binding) in &mut self.0 {
            if *other != input && binding.button() == Some(button) {
                binding.button = None;
            }
        }
        let mut binding = self.get(input);
        binding.button = Some(button_name(button).to_owned());
        self.set(input, binding);
    }

    /// Leave `input` with no key, or no pad button.
    pub fn clear(&mut self, input: DsInput, device: Device) {
        let mut binding = self.get(input);
        match device {
            Device::Keyboard => binding.key = None,
            Device::Pad => binding.button = None,
        }
        self.set(input, binding);
    }

    /// The DS key mask the keyboard is holding.
    #[must_use]
    pub fn key_mask(&self, input: &egui::InputState) -> u32 {
        self.entries()
            .filter(|(_, binding)| binding.key().is_some_and(|key| input.key_down(key)))
            .fold(0, |mask, (it, _)| mask | it.bit())
    }

    /// The bit `button` contributes to the key mask, or 0 if nothing is bound
    /// to it.
    ///
    /// The event-driven counterpart of [`Self::pad_mask`]: that one asks the
    /// pad what is *held*, this one turns a press that has already been and
    /// gone into the same mask. See [`crate::pad`]'s Latency note.
    #[must_use]
    pub fn button_bit(&self, button: gilrs::Button) -> u32 {
        self.entries()
            .find(|(_, binding)| binding.button() == Some(button))
            .map_or(0, |(input, _)| input.bit())
    }

    /// Whether any DS button answers to `button`.
    #[must_use]
    pub fn is_bound(&self, button: gilrs::Button) -> bool {
        self.button_bit(button) != 0
    }

    /// The DS key mask one pad is holding.
    #[must_use]
    pub fn pad_mask(&self, pad: &gilrs::Gamepad<'_>) -> u32 {
        self.entries()
            .filter(|(_, binding)| binding.button().is_some_and(|b| pad.is_pressed(b)))
            .fold(0, |mask, (it, _)| mask | it.bit())
    }
}

/// Which half of a binding is being edited.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Device {
    Keyboard,
    Pad,
}

/// Every pad button the dialog offers, and the name it is stored under.
///
/// gilrs's own names, so a file says `East` where gilrs says `East`. `C`, `Z`
/// and `Mode` are left out: the first two exist for six-button arcade pads and
/// the third is the guide button, which the operating system usually takes.
pub const PAD_BUTTONS: &[(gilrs::Button, &str)] = &[
    (gilrs::Button::South, "South"),
    (gilrs::Button::East, "East"),
    (gilrs::Button::North, "North"),
    (gilrs::Button::West, "West"),
    (gilrs::Button::LeftTrigger, "LeftTrigger"),
    (gilrs::Button::LeftTrigger2, "LeftTrigger2"),
    (gilrs::Button::RightTrigger, "RightTrigger"),
    (gilrs::Button::RightTrigger2, "RightTrigger2"),
    (gilrs::Button::Select, "Select"),
    (gilrs::Button::Start, "Start"),
    (gilrs::Button::LeftThumb, "LeftThumb"),
    (gilrs::Button::RightThumb, "RightThumb"),
    (gilrs::Button::DPadUp, "DPadUp"),
    (gilrs::Button::DPadDown, "DPadDown"),
    (gilrs::Button::DPadLeft, "DPadLeft"),
    (gilrs::Button::DPadRight, "DPadRight"),
];

/// The name `button` is stored under, or `"Unknown"` for one the dialog does
/// not offer.
#[must_use]
pub fn button_name(button: gilrs::Button) -> &'static str {
    PAD_BUTTONS.iter().find(|(it, _)| *it == button).map_or("Unknown", |(_, name)| *name)
}

/// The button a stored name means, or nothing.
#[must_use]
pub fn button_from_name(name: &str) -> Option<gilrs::Button> {
    PAD_BUTTONS.iter().find(|(_, it)| *it == name).map(|(button, _)| *button)
}

#[cfg(test)]
mod tests {
    use super::{Binding, Bindings, Device, DsInput, PAD_BUTTONS, button_from_name, button_name};

    /// The event-latched half of pad input: a press that has already ended has
    /// to reach the same bit `pad_mask` would have produced for it while held.
    /// See [`crate::pad`]'s Latency note.
    #[test]
    fn a_pad_button_maps_to_the_bit_its_binding_names() {
        let bindings = Bindings::default();
        assert_eq!(bindings.button_bit(gilrs::Button::East), DsInput::A.bit());
        assert_eq!(bindings.button_bit(gilrs::Button::South), DsInput::B.bit());
        assert_eq!(bindings.button_bit(gilrs::Button::Start), DsInput::Start.bit());
        assert_eq!(bindings.button_bit(gilrs::Button::LeftThumb), 0, "nothing is bound to it");
    }

    /// The speed button steps the emulation only while it is free -- a user who
    /// binds it to a DS button gets the DS button.
    #[test]
    fn the_speed_button_is_free_until_it_is_bound() {
        let mut bindings = Bindings::default();
        assert!(!bindings.is_bound(crate::pad::SPEED_BUTTON));
        bindings.bind_button(DsInput::Select, crate::pad::SPEED_BUTTON);
        assert!(bindings.is_bound(crate::pad::SPEED_BUTTON));
        assert_eq!(bindings.button_bit(crate::pad::SPEED_BUTTON), DsInput::Select.bit());
    }

    /// Every default has to survive being written out and read back, or the
    /// first save would silently unbind the controls.
    #[test]
    fn the_defaults_round_trip_through_their_names() {
        let bindings = Bindings::default();
        for (input, binding) in bindings.entries() {
            assert!(binding.key().is_some(), "{input:?} lost its key");
            assert!(binding.button().is_some(), "{input:?} lost its pad button");
        }
        let json = serde_json::to_string(&bindings).expect("serialisable");
        let back: Bindings = serde_json::from_str(&json).expect("readable");
        assert_eq!(back, bindings);
    }

    /// The defaults are melonDS's, and getting one wrong is the sort of thing
    /// nobody notices until a game will not start.
    #[test]
    fn the_defaults_are_the_ones_melonds_ships() {
        let bindings = Bindings::default();
        for (input, key, button) in [
            (DsInput::A, "X", "East"),
            (DsInput::B, "Z", "South"),
            (DsInput::X, "S", "North"),
            (DsInput::Y, "A", "West"),
            (DsInput::Start, "Enter", "Start"),
            (DsInput::Select, "Backspace", "Select"),
            // egui spells the arrow keys without the prefix its variants carry;
            // this is the name that goes in the file, so it is the one checked.
            (DsInput::Up, "Up", "DPadUp"),
        ] {
            let binding = bindings.get(input);
            assert_eq!(binding.key.as_deref(), Some(key), "{input:?}");
            assert_eq!(binding.button.as_deref(), Some(button), "{input:?}");
        }
    }

    /// Two DS buttons on one key is never intended: the second shadows the
    /// first, and the dialog would show a map that does not behave as it reads.
    #[test]
    fn binding_a_key_takes_it_off_whatever_had_it() {
        let mut bindings = Bindings::default();
        // `Z` is B by default; give it to A instead.
        bindings.bind_key(DsInput::A, egui::Key::Z);
        assert_eq!(bindings.get(DsInput::A).key.as_deref(), Some("Z"));
        assert_eq!(bindings.get(DsInput::B).key, None, "B kept a key that moved to A");
        // The pad half is untouched by a keyboard rebind.
        assert_eq!(bindings.get(DsInput::B).button.as_deref(), Some("South"));
    }

    #[test]
    fn binding_a_pad_button_takes_it_off_whatever_had_it() {
        let mut bindings = Bindings::default();
        bindings.bind_button(DsInput::L, gilrs::Button::East);
        assert_eq!(bindings.get(DsInput::L).button.as_deref(), Some("East"));
        assert_eq!(bindings.get(DsInput::A).button, None);
    }

    #[test]
    fn clearing_leaves_the_other_half_alone() {
        let mut bindings = Bindings::default();
        bindings.clear(DsInput::A, Device::Keyboard);
        assert_eq!(bindings.get(DsInput::A).key, None);
        assert_eq!(bindings.get(DsInput::A).button.as_deref(), Some("East"));
    }

    /// A file from another version must leave the emulator startable, with the
    /// names it does not know simply unbound.
    #[test]
    fn an_unknown_name_reads_as_unbound() {
        let json = r#"[["a", {"key": "NoSuchKey", "button": "NoSuchButton"}]]"#;
        let bindings: Bindings = serde_json::from_str(json).expect("readable");
        let binding = bindings.get(DsInput::A);
        assert!(binding.key().is_none() && binding.button().is_none());
        // And a button the file omits entirely.
        assert_eq!(bindings.get(DsInput::Start), Binding::default());
    }

    #[test]
    fn every_offered_pad_button_round_trips() {
        for (button, name) in PAD_BUTTONS {
            assert_eq!(button_name(*button), *name);
            assert_eq!(button_from_name(name), Some(*button));
        }
        assert_eq!(button_from_name("nonsense"), None);
        assert_eq!(button_name(gilrs::Button::Mode), "Unknown", "a button not offered");
    }

    /// No two DS buttons may share a key or a button out of the box.
    #[test]
    fn the_defaults_do_not_collide() {
        let bindings = Bindings::default();
        let mut keys: Vec<String> = Vec::new();
        let mut buttons: Vec<String> = Vec::new();
        for (_, binding) in bindings.entries() {
            if let Some(key) = binding.key {
                assert!(!keys.contains(&key), "{key} is bound twice");
                keys.push(key);
            }
            if let Some(button) = binding.button {
                assert!(!buttons.contains(&button), "{button} is bound twice");
                buttons.push(button);
            }
        }
        assert_eq!(keys.len(), DsInput::ALL.len());
        assert_eq!(buttons.len(), DsInput::ALL.len());
    }
}
