//! The menu bar, built to melonDS's shape.
//!
//! Structure, wording and offered values are transcribed from melonDS's
//! `frontend/qt_sdl/Window.cpp` (the `menubar->addMenu(...)` blocks), so that
//! someone who knows melonDS finds the same commands in the same places.
//!
//! Every entry that *can* be backed is live. The rest are **shown but disabled**
//! rather than omitted, each carrying the specific reason it cannot work — see
//! [`Unavailable`]. The shape then matches melonDS's, and what is missing is
//! visible in the UI instead of merely absent.

use egui::Ui;

use crate::{
    app::{MelonEgui, Pane, RECENT_LIMIT, STATE_SLOTS},
    view::{AspectRatio, Rotation, SCREEN_GAPS, ScreenLayout, ScreenSizing},
};

/// Why a menu entry is disabled.
#[derive(Clone, Copy)]
enum Unavailable {
    /// The melonDS core can do it, but `melonds-rs`'s FFI (`shim.h`) exposes no
    /// entry point for it, so no front end built on these bindings can reach it.
    Bindings,
    /// Reachable through the bindings, but not built yet.
    Planned,
}

impl Unavailable {
    const fn reason(self) -> &'static str {
        match self {
            Self::Bindings => {
                "Not reachable: the melonds-rs bindings expose no FFI entry point for this."
            }
            Self::Planned => "Not implemented yet (the bindings do support it).",
        }
    }
}

/// What a menu entry asks for. Returned rather than acted on so that the menu
/// closure does not need `&mut` access to the app while egui holds it.
pub enum Action {
    OpenRom,
    /// One of the remembered ROMs, by index into the recent list.
    OpenRecent(usize),
    ClearRecent,
    InsertCart,
    EjectCart,
    ImportSavefile,
    /// `Some(slot)` for one of the numbered slots, `None` for "File...".
    SaveState(Option<u8>),
    LoadState(Option<u8>),
    UndoStateLoad,
    /// Reveal the directory this front end keeps its files in.
    OpenDirectory,
    Quit,
    TogglePause,
    Reset,
    Stop,
    FrameStep,
    /// Resize the window so the screens land on exactly this scale.
    ScreenSize(f32),
    /// A second window showing the same console.
    NewWindow,
    /// Open (or close) a second console on the shared airwaves.
    LaunchInstance,
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

/// An entry that is present for shape but cannot be used, with the reason on
/// hover.
fn unavailable(ui: &mut Ui, label: &str, why: Unavailable) {
    ui.add_enabled(false, egui::Button::new(label)).on_disabled_hover_text(why.reason());
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
        ui.menu_button("Open recent", |ui| {
            let recents = app.recent_roms().to_vec();
            if recents.is_empty() {
                ui.add_enabled(false, egui::Button::new("(nothing yet)"));
            }
            for (i, path) in recents.iter().take(RECENT_LIMIT).enumerate() {
                // Numbered as melonDS numbers them, and labelled by file name so
                // the list stays readable with long paths.
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let label = format!("{}.  {name}", i + 1);
                let clicked = ui
                    .add(egui::Button::new(&label))
                    .on_hover_text(path.display().to_string())
                    .clicked();
                if clicked {
                    ui.close();
                    action = Some(Action::OpenRecent(i));
                }
            }
            if !recents.is_empty() {
                ui.separator();
                action = action.take().or_else(|| entry(ui, true, "Clear", Action::ClearRecent));
            }
        });
        // Booting the firmware needs a firmware image and a boot path the shim
        // does not offer: `mds_boot` always direct-boots a cart with FreeBIOS.
        unavailable(ui, "Boot firmware", Unavailable::Bindings);
        ui.separator();

        ui.label(format!("DS slot: {}", app.cart_label()));
        action = action.take().or_else(|| entry(ui, true, "Insert cart...", Action::InsertCart));
        action = action.take().or_else(|| entry(ui, loaded, "Eject cart", Action::EjectCart));
        ui.separator();

        // There is no GBA slot in the FFI: `mds_nds_new` takes one ROM.
        ui.label("GBA slot: (none)");
        unavailable(ui, "Insert ROM cart...", Unavailable::Bindings);
        unavailable(ui, "Insert add-on cart", Unavailable::Bindings);
        unavailable(ui, "Eject cart", Unavailable::Bindings);
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

        action = action
            .take()
            .or_else(|| entry(ui, true, "Open melon_egui directory", Action::OpenDirectory));
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

        action = action
            .take()
            .or_else(|| entry(ui, loaded, "Power management", Action::TogglePane(Pane::Power)));
        action = action
            .take()
            .or_else(|| entry(ui, loaded, "Date and time", Action::TogglePane(Pane::DateTime)));
        ui.separator();

        // melonDS's AR engine, running the codes from the ARM7's VBlank
        // handler exactly as the hardware does.
        let mut cheats_on = app.cheats_enabled;
        if ui.checkbox(&mut cheats_on, "Enable cheats").clicked() {
            app.cheats_enabled = cheats_on;
        }
        action = action
            .take()
            .or_else(|| entry(ui, true, "Setup cheat codes", Action::TogglePane(Pane::Cheats)));
        ui.separator();

        action = action
            .take()
            .or_else(|| entry(ui, loaded, "ROM info", Action::TogglePane(Pane::RomInfo)));
        action = action
            .take()
            .or_else(|| entry(ui, loaded, "RAM search", Action::TogglePane(Pane::RamSearch)));
        // No DSi mode in this build at all.
        unavailable(ui, "Manage DSi titles", Unavailable::Bindings);
        ui.separator();

        ui.menu_button("Multiplayer", |ui| {
            let label =
                if app.has_guest() { "Close second instance" } else { "Launch new instance" };
            action = action.take().or_else(|| entry(ui, loaded, label, Action::LaunchInstance));
            action = action
                .take()
                .or_else(|| entry(ui, true, "Wireless status", Action::TogglePane(Pane::Wireless)));
            ui.separator();
            // LAN carries the same frames over a real network. The airwaves are
            // in-process only, so this is a transport that does not exist yet
            // rather than anything the bindings withhold.
            unavailable(ui, "Host LAN game", Unavailable::Planned);
            unavailable(ui, "Join LAN game", Unavailable::Planned);
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
                ui.radio_value(&mut view.layout, layout, layout.label());
            }
            ui.separator();
            ui.checkbox(&mut view.swap, "Swap screens");
        });
        ui.menu_button("Screen sizing", |ui| {
            for sizing in ScreenSizing::ALL {
                ui.radio_value(&mut view.sizing, sizing, sizing.label());
            }
            ui.separator();
            ui.checkbox(&mut view.integer_scaling, "Force integer scaling");
        });
        ui.menu_button("Aspect ratio", |ui| {
            // Per screen, and labelled per screen, exactly as melonDS lists it.
            for aspect in AspectRatio::ALL {
                ui.radio_value(&mut view.aspect_top, aspect, format!("Top {}", aspect.label()));
            }
            ui.separator();
            for aspect in AspectRatio::ALL {
                ui.radio_value(
                    &mut view.aspect_bottom,
                    aspect,
                    format!("Bottom {}", aspect.label()),
                );
            }
        });
        ui.separator();

        action = action.take().or_else(|| entry(ui, true, "Open new window", Action::NewWindow));
        ui.separator();

        let view = &mut app.view;
        ui.checkbox(&mut view.filtering, "Screen filtering");
        ui.checkbox(&mut view.show_osd, "Show OSD");
    });
    action
}

fn config_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button("Config", |ui| {
        action = action
            .take()
            .or_else(|| entry(ui, true, "Emu settings", Action::TogglePane(Pane::EmuSettings)));
        action = action
            .take()
            .or_else(|| entry(ui, true, "Preferences...", Action::TogglePane(Pane::Preferences)));
        ui.separator();

        action = action
            .take()
            .or_else(|| entry(ui, true, "Input and hotkeys", Action::TogglePane(Pane::Input)));
        action = action
            .take()
            .or_else(|| entry(ui, true, "Video settings", Action::TogglePane(Pane::VideoSettings)));
        // No camera in the FFI.
        unavailable(ui, "Camera settings", Unavailable::Bindings);
        action = action
            .take()
            .or_else(|| entry(ui, true, "Audio settings", Action::TogglePane(Pane::AudioSettings)));
        action = action.take().or_else(|| {
            entry(ui, true, "Multiplayer settings", Action::TogglePane(Pane::Wireless))
        });
        action = action
            .take()
            .or_else(|| entry(ui, true, "Wifi settings", Action::TogglePane(Pane::Wireless)));
        // The firmware is generated by the shim; its contents are not settable.
        unavailable(ui, "Firmware settings", Unavailable::Bindings);
        action = action
            .take()
            .or_else(|| entry(ui, true, "Interface settings", Action::TogglePane(Pane::Interface)));
        action = action
            .take()
            .or_else(|| entry(ui, true, "Path settings", Action::TogglePane(Pane::Paths)));
        ui.separator();

        ui.checkbox(&mut app.limit_framerate, "Limit framerate");
        let has_audio = app.has_audio();
        ui.add_enabled(has_audio, egui::Checkbox::new(&mut app.audio_sync, "Audio sync"))
            .on_disabled_hover_text("No audio output device.");
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
