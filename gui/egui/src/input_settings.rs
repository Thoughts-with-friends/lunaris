//! "Input Settings" window: remapping of NDS buttons to host keys and gamepad
//! buttons, staged in a draft and written to `config.json` by "Apply".
//!
//! The editor exposes one keyboard binding and one gamepad-button binding per
//! NDS button, which is what [`InputBinding`] entries with a single source
//! express. Analog-axis bindings and multi-source chords (`Ctrl + L`) have no
//! row here; they are carried through untouched so hand-authored `config.json`
//! entries and the default stick-as-D-pad mapping survive an "Apply". Chord
//! authoring stays a `config.json`-only feature.

use eframe::egui;
use lunaris_gui_common::{
    config::Config,
    input::{
        enums::{BindKey, GamepadButton, InputBinding, InputSource, KeyboardKey},
        input_default::default_input_bindings,
    },
};

/// NDS buttons in the order they are listed in the window.
const BIND_KEYS: [BindKey; 12] = [
    BindKey::Up,
    BindKey::Down,
    BindKey::Left,
    BindKey::Right,
    BindKey::A,
    BindKey::B,
    BindKey::X,
    BindKey::Y,
    BindKey::L,
    BindKey::R,
    BindKey::Start,
    BindKey::Select,
];

/// Which device the pending capture is listening to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptureKind {
    Keyboard,
    Gamepad,
}

/// One editable row: the host inputs currently assigned to `target`.
struct BindingRow {
    target: BindKey,
    keyboard: Option<KeyboardKey>,
    gamepad: Option<GamepadButton>,
}

/// What [`InputSettingsState::show`] did to the config this repaint.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputSettingsAction {
    None,
    /// The draft was written to [`Config::input_bindings`]. The caller must
    /// rebuild whatever it derived from the old bindings.
    Applied,
}

#[derive(Default)]
pub struct InputSettingsState {
    open: bool,
    /// Editable rows, one per [`BIND_KEYS`] entry. Empty until first opened.
    draft: Vec<BindingRow>,
    /// Bindings the row editor cannot represent, re-emitted verbatim on apply.
    preserved: Vec<InputBinding>,
    capture: Option<(BindKey, CaptureKind)>,
}

impl InputSettingsState {
    /// Opens the window, discarding any uncommitted draft in favour of what is
    /// currently configured.
    pub fn open(&mut self, config: &Config) {
        self.load(&config.input_bindings);
        self.capture = None;
        self.open = true;
    }

    /// Whether a capture is waiting for a key or button press.
    ///
    /// The caller must stop feeding host input to the emulator while this is
    /// true, otherwise pressing `A` to rebind it also presses NDS A.
    pub fn is_capturing(&self) -> bool {
        self.capture.is_some()
    }

    /// Adds the window's entry to a menu.
    pub fn menu_item(&mut self, ui: &mut egui::Ui, config: &Config) {
        if ui.button("Input and hotkeys").clicked() {
            self.open(config);
            ui.close();
        }
    }

    /// Splits configured bindings into editable rows plus the ones this editor
    /// leaves alone. The first single-source binding per device wins the row;
    /// any further one is preserved rather than dropped.
    fn load(&mut self, bindings: &[InputBinding]) {
        self.draft = BIND_KEYS
            .iter()
            .map(|target| BindingRow { target: *target, keyboard: None, gamepad: None })
            .collect();
        self.preserved.clear();

        for binding in bindings {
            let single = match binding.sources.as_slice() {
                [source] => Some(source),
                _ => None,
            };
            let row = self.draft.iter_mut().find(|row| row.target == binding.target);

            match (single, row) {
                (Some(InputSource::Keyboard { key }), Some(row)) if row.keyboard.is_none() => {
                    row.keyboard = Some(*key);
                }
                (Some(InputSource::GamepadButton { button }), Some(row))
                    if row.gamepad.is_none() =>
                {
                    row.gamepad = Some(*button);
                }
                _ => self.preserved.push(binding.clone()),
            }
        }
    }

    /// Rebuilds the full binding list from the draft plus the preserved
    /// entries.
    fn to_bindings(&self) -> Vec<InputBinding> {
        let mut bindings = Vec::with_capacity(self.draft.len() * 2 + self.preserved.len());
        for row in &self.draft {
            if let Some(key) = row.keyboard {
                bindings.push(InputBinding {
                    sources: vec![InputSource::Keyboard { key }],
                    target: row.target,
                });
            }
            if let Some(button) = row.gamepad {
                bindings.push(InputBinding {
                    sources: vec![InputSource::GamepadButton { button }],
                    target: row.target,
                });
            }
        }
        bindings.extend(self.preserved.iter().cloned());
        bindings
    }

    /// Consumes a pending capture from this repaint's host input.
    ///
    /// The gamepad branch drains the same `gilrs` queue the emulator input
    /// path drains, which is skipped entirely while capturing, so one press is
    /// never both bound here and delivered to the emulator.
    pub fn poll_capture(&mut self, ctx: &egui::Context, gilrs: &mut gilrs::Gilrs) {
        let Some((target, kind)) = self.capture else { return };

        match kind {
            CaptureKind::Keyboard => {
                let pressed = ctx.input(|i| {
                    i.events.iter().find_map(|event| match event {
                        egui::Event::Key { key, pressed: true, .. } => Some(*key),
                        _ => None,
                    })
                });
                let Some(key) = pressed else { return };

                match key {
                    egui::Key::Escape => self.capture = None,
                    egui::Key::Backspace | egui::Key::Delete => {
                        self.set_keyboard(target, None);
                        self.capture = None;
                    }
                    // An unmappable key leaves the capture running rather than
                    // silently binding the `Unknown` placeholder.
                    key => {
                        let key = crate::input::egui_to_config_keyboard_key(key);
                        if key != KeyboardKey::Unknown {
                            self.set_keyboard(target, Some(key));
                            self.capture = None;
                        }
                    }
                }
            }
            CaptureKind::Gamepad => {
                while let Some(event) = gilrs.next_event() {
                    if let gilrs::EventType::ButtonPressed(button, _) = event.event {
                        let button = crate::input::egui_to_config_gamepad_button(button);
                        self.set_gamepad(target, Some(button));
                        self.capture = None;
                        break;
                    }
                }
            }
        }
    }

    /// Assigns a keyboard key, clearing it from whichever other button held it
    /// so one key can never drive two NDS buttons at once.
    fn set_keyboard(&mut self, target: BindKey, key: Option<KeyboardKey>) {
        for row in &mut self.draft {
            if row.target == target {
                row.keyboard = key;
            } else if key.is_some() && row.keyboard == key {
                row.keyboard = None;
            }
        }
    }

    /// Gamepad-button counterpart of [`Self::set_keyboard`].
    fn set_gamepad(&mut self, target: BindKey, button: Option<GamepadButton>) {
        for row in &mut self.draft {
            if row.target == target {
                row.gamepad = button;
            } else if button.is_some() && row.gamepad == button {
                row.gamepad = None;
            }
        }
    }

    /// Draws the window. Nothing reaches `config` until "Apply" is pressed.
    pub fn show(&mut self, ctx: &egui::Context, config: &mut Config) -> InputSettingsAction {
        if !self.open {
            return InputSettingsAction::None;
        }

        let mut open = self.open;
        let mut applied = false;

        egui::Window::new("Input and hotkeys").open(&mut open).default_size([380.0, 460.0]).show(
            ctx,
            |ui| {
                egui::Grid::new("input_settings_grid").num_columns(3).striped(true).show(
                    ui,
                    |ui| {
                        ui.strong("NDS button");
                        ui.strong("Keyboard");
                        ui.strong("Gamepad");
                        ui.end_row();

                        // Copied out so the row loop's borrow of `draft` and
                        // the capture writes below stay independent.
                        let capture = self.capture;
                        let mut next_capture = capture;

                        for row in &self.draft {
                            ui.label(format!("{:?}", row.target));

                            let waiting = |kind| capture == Some((row.target, kind));
                            let label =
                                |waiting: bool, bound: Option<String>| match (waiting, bound) {
                                    (true, _) => "...".to_owned(),
                                    (false, Some(name)) => name,
                                    (false, None) => "-".to_owned(),
                                };

                            let keyboard = row.keyboard.map(|key| format!("{key:?}"));
                            if ui.button(label(waiting(CaptureKind::Keyboard), keyboard)).clicked()
                            {
                                next_capture = Some((row.target, CaptureKind::Keyboard));
                            }

                            let gamepad = row.gamepad.map(|button| format!("{button:?}"));
                            if ui.button(label(waiting(CaptureKind::Gamepad), gamepad)).clicked() {
                                next_capture = Some((row.target, CaptureKind::Gamepad));
                            }
                            ui.end_row();
                        }

                        self.capture = next_capture;
                    },
                );

                ui.separator();

                if self.capture.is_some() {
                    ui.label(
                        "Press a key or gamepad button to assign. Esc cancels, Backspace clears.",
                    );
                } else {
                    ui.weak("Click a cell to reassign it. Changes take effect on Apply.");
                }

                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        config.input_bindings = self.to_bindings();
                        applied = true;
                    }
                    // A bad rebind can leave the keyboard unable to play; this
                    // is the way back, and it is staged like every other edit.
                    if ui.button("Reset to defaults").clicked() {
                        self.load(&default_input_bindings());
                        self.capture = None;
                    }
                });

                if !self.preserved.is_empty() {
                    ui.weak(format!(
                        "{} binding(s) not shown (analog axes and multi-key chords) are kept \
                         as-is.",
                        self.preserved.len()
                    ));
                }
            },
        );

        self.open = open;
        if !self.open {
            self.capture = None;
        }

        if applied { InputSettingsAction::Applied } else { InputSettingsAction::None }
    }
}
