//! The menu bar, built to melonDS's shape.
//!
//! Structure, wording and offered values are transcribed from melonDS's
//! `frontend/qt_sdl/Window.cpp` (the `menubar->addMenu(...)` blocks), so that
//! someone who knows melonDS finds the same commands in the same places.
//!
//! Entries this front end does not implement are **shown but disabled** rather
//! than omitted: the shape then matches melonDS's, and what is missing is
//! visible instead of merely absent. Each disabled group carries a tooltip
//! saying why.

use egui::Ui;

use crate::{
    app::{MelonEgui, Pane, STATE_SLOTS},
    view::{Rotation, SCREEN_GAPS, ScreenLayout, ScreenSizing},
};

/// What a menu entry asks for. Returned rather than acted on so that the menu
/// closure does not need `&mut` access to the app while egui holds it.
pub enum Action {
    OpenRom,
    InsertCart,
    EjectCart,
    ImportSavefile,
    /// `Some(slot)` for one of the numbered slots, `None` for "File...".
    SaveState(Option<u8>),
    LoadState(Option<u8>),
    UndoStateLoad,
    Quit,
    TogglePause,
    Reset,
    Stop,
    FrameStep,
    /// Resize the window so the screens land on exactly this scale.
    ScreenSize(f32),
    /// Show or hide one of the auxiliary windows.
    TogglePane(Pane),
}

/// Draw the bar, returning whichever entry was clicked.
pub fn bar(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    egui::MenuBar::new().ui(ui, |ui| {
        action = file_menu(app, ui)
            .or_else(|| system_menu(app, ui))
            .or_else(|| view_menu(app, ui))
            .or_else(|| config_menu(app, ui))
            .or_else(|| help_menu(ui));
    });
    action
}

/// A menu entry that is present for shape but has nothing behind it.
fn unimplemented(ui: &mut Ui, label: &str) {
    ui.add_enabled(false, egui::Button::new(label))
        .on_disabled_hover_text("not implemented in melon_egui");
}

/// An entry that runs `action` and closes the menu.
fn entry(ui: &mut Ui, enabled: bool, label: &str, action: Action) -> Option<Action> {
    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        ui.close();
        return Some(action);
    }
    None
}

fn file_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button("File", |ui| {
        let loaded = app.is_loaded();

        action = action.take().or_else(|| entry(ui, true, "Open ROM...", Action::OpenRom));
        unimplemented(ui, "Open recent");
        // Booting the firmware needs a real firmware image; this build's shim
        // always direct-boots a cart with FreeBIOS.
        unimplemented(ui, "Boot firmware");
        ui.separator();

        ui.label(format!("DS slot: {}", app.cart_label()));
        action = action.take().or_else(|| entry(ui, true, "Insert cart...", Action::InsertCart));
        action = action.take().or_else(|| entry(ui, loaded, "Eject cart", Action::EjectCart));
        ui.separator();

        // No GBA slot: the bindings expose no second cart.
        ui.label("GBA slot: (none)");
        unimplemented(ui, "Insert ROM cart...");
        unimplemented(ui, "Insert add-on cart");
        unimplemented(ui, "Eject cart");
        ui.separator();

        action =
            action.take().or_else(|| entry(ui, loaded, "Import savefile", Action::ImportSavefile));
        ui.separator();

        ui.menu_button("Save state", |ui| {
            for slot in 1..=STATE_SLOTS {
                action = action.take().or_else(|| {
                    entry(ui, loaded, &slot.to_string(), Action::SaveState(Some(slot)))
                });
            }
            ui.separator();
            action =
                action.take().or_else(|| entry(ui, loaded, "File...", Action::SaveState(None)));
        });
        ui.menu_button("Load state", |ui| {
            for slot in 1..=STATE_SLOTS {
                let exists = app.state_slot_exists(slot);
                action = action.take().or_else(|| {
                    entry(ui, loaded && exists, &slot.to_string(), Action::LoadState(Some(slot)))
                });
            }
            ui.separator();
            action =
                action.take().or_else(|| entry(ui, loaded, "File...", Action::LoadState(None)));
        });
        action = action.take().or_else(|| {
            entry(ui, app.can_undo_state_load(), "Undo state load", Action::UndoStateLoad)
        });
        ui.separator();

        // No config directory: this front end keeps nothing outside the ROM's
        // own folder.
        unimplemented(ui, "Open melonDS directory");
        ui.separator();

        action = action.take().or_else(|| entry(ui, true, "Quit", Action::Quit));
    });
    action
}

fn system_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button("System", |ui| {
        let loaded = app.is_loaded();

        // Pause is a checkbox in melonDS, and reads as one here too.
        let mut paused = app.is_paused();
        if ui.add_enabled(loaded, egui::Checkbox::new(&mut paused, "Pause")).clicked() {
            ui.close();
            action = Some(Action::TogglePause);
        }
        action = action.take().or_else(|| entry(ui, loaded, "Reset", Action::Reset));
        action = action.take().or_else(|| entry(ui, loaded, "Stop", Action::Stop));
        action = action.take().or_else(|| entry(ui, loaded, "Frame step", Action::FrameStep));
        ui.separator();

        // Both need core settings the bindings do not surface.
        unimplemented(ui, "Power management");
        unimplemented(ui, "Date and time");
        ui.separator();

        unimplemented(ui, "Enable cheats");
        unimplemented(ui, "Setup cheat codes");
        ui.separator();

        action = action
            .take()
            .or_else(|| entry(ui, loaded, "ROM info", Action::TogglePane(Pane::RomInfo)));
        unimplemented(ui, "RAM search");
        unimplemented(ui, "Manage DSi titles");
        ui.separator();

        ui.menu_button("Multiplayer", |ui| {
            // The whole point of comparing against melonDS, and the one thing
            // this front end deliberately does not do yet: it runs a single
            // unlinked console, see `emu.rs`.
            unimplemented(ui, "Launch new instance");
            ui.separator();
            unimplemented(ui, "Host LAN game");
            unimplemented(ui, "Join LAN game");
        });
    });
    action
}

fn view_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button("View", |ui| {
        ui.menu_button("Screen size", |ui| {
            for scale in 1..=4 {
                action = action.take().or_else(|| {
                    entry(ui, true, &format!("{scale}x"), Action::ScreenSize(scale as f32))
                });
            }
        });

        let view = &mut app.view;
        ui.menu_button("Screen rotation", |ui| {
            for rotation in Rotation::ALL {
                ui.radio_value(&mut view.rotation, rotation, format!("{}°", rotation.degrees()));
            }
        });
        ui.menu_button("Screen gap", |ui| {
            for gap in SCREEN_GAPS {
                ui.radio_value(&mut view.gap, gap, format!("{gap} px"));
            }
        });
        ui.menu_button("Screen layout", |ui| {
            for layout in ScreenLayout::ALL {
                if layout.supported() {
                    ui.radio_value(&mut view.layout, layout, layout.label());
                } else {
                    unimplemented(ui, layout.label());
                }
            }
            ui.separator();
            ui.checkbox(&mut view.swap, "Swap screens");
        });
        ui.menu_button("Screen sizing", |ui| {
            for sizing in ScreenSizing::ALL {
                if sizing.supported() {
                    ui.radio_value(&mut view.sizing, sizing, sizing.label());
                } else {
                    unimplemented(ui, sizing.label());
                }
            }
            ui.separator();
            ui.checkbox(&mut view.integer_scaling, "Force integer scaling");
        });
        // Non-native aspect ratios would need the layout to stretch rather than
        // fit, which is the opposite of what a reference picture wants.
        unimplemented(ui, "Aspect ratio");
        ui.separator();

        // One window, one console; see the Multiplayer note above.
        unimplemented(ui, "Open new window");
        ui.separator();

        ui.checkbox(&mut view.filtering, "Screen filtering");
        ui.checkbox(&mut view.show_osd, "Show OSD");
    });
    action
}

fn config_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button("Config", |ui| {
        // Everything melonDS keeps in a settings dialog needs config plumbing
        // this front end does not have; the input list is the exception because
        // its bindings are fixed and worth being able to read.
        unimplemented(ui, "Emu settings");
        unimplemented(ui, "Preferences...");
        ui.separator();

        action = action
            .take()
            .or_else(|| entry(ui, true, "Input and hotkeys", Action::TogglePane(Pane::Input)));
        unimplemented(ui, "Video settings");
        unimplemented(ui, "Camera settings");
        // There is no audio output at all yet, so nothing to configure.
        unimplemented(ui, "Audio settings");
        unimplemented(ui, "Multiplayer settings");
        unimplemented(ui, "Wifi settings");
        unimplemented(ui, "Firmware settings");
        unimplemented(ui, "Interface settings");
        unimplemented(ui, "Path settings");
        ui.separator();

        ui.checkbox(&mut app.limit_framerate, "Limit framerate");
        unimplemented(ui, "Audio sync");
    });
    action
}

fn help_menu(ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button("Help", |ui| {
        action = entry(ui, true, "About...", Action::TogglePane(Pane::About));
    });
    action
}
