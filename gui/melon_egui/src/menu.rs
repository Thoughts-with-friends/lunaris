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
    i18n::I18nKey as K,
    view::{AspectRatio, Rotation, SCREEN_GAPS, ScreenLayout, ScreenSizing},
};

/// Why a menu entry is disabled.
#[derive(Clone, Copy)]
enum Unavailable {
    /// The melonDS core can do it, but `melonds-rs`'s FFI (`shim.h`) exposes no
    /// entry point for it, so no front end built on these bindings can reach it.
    Bindings,
}

impl Unavailable {
    /// The key whose text explains this, so the reason is translated along with
    /// everything else rather than being the one English string left on a
    /// Japanese menu.
    const fn key(self) -> K {
        match self {
            Self::Bindings => K::UnavailableBindings,
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
    /// Accept one remote LAN console on the configured UDP port.
    HostLanGame,
    /// Connect this console to the configured LAN host.
    GuestLanGame,
    /// Run both consoles here and stream the second one out. See
    /// [`crate::remote`].
    HostRemoteDesktop,
    /// Become a screen for a console running elsewhere.
    JoinRemoteDesktop,
    /// End whichever Remote Desktop session is running.
    StopRemoteDesktop,
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
            .or_else(|| help_menu(app, ui));
    });
    action
}

/// An entry that is present for shape but cannot be used, with the reason on
/// hover.
///
/// Takes the app so both the label and the reason come out of the translation
/// map; a menu with one English tooltip on it reads as an oversight.
fn unavailable(app: &MelonEgui, ui: &mut Ui, label: K, why: Unavailable) {
    let (label, reason) = (app.i18n().s(label), app.i18n().s(why.key()));
    ui.add_enabled(false, egui::Button::new(label)).on_disabled_hover_text(reason);
}

/// An entry that runs `action` and closes the menu.
fn entry(ui: &mut Ui, enabled: bool, label: &str, action: Action) -> Option<Action> {
    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        ui.close();
        return Some(action);
    }
    None
}

/// A translated entry, which is what nearly every entry here is.
///
/// The label is copied out of the map before the widget is built: `app.i18n`
/// borrows `app`, and several call sites need `&mut app` in the same
/// expression.
fn item(app: &MelonEgui, ui: &mut Ui, enabled: bool, label: K, action: Action) -> Option<Action> {
    entry(ui, enabled, &app.i18n().s(label), action)
}

fn file_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;

    ui.menu_button(app.i18n().s(K::FileLabel), |ui| {
        let loaded = app.is_loaded();

        action = action.take().or_else(|| item(app, ui, true, K::OpenRom, Action::OpenRom));
        ui.menu_button(app.i18n().s(K::OpenRecent), |ui| {
            let recents = app.recent_roms().to_vec();
            if recents.is_empty() {
                ui.add_enabled(false, egui::Button::new(app.i18n().s(K::NothingYet)));
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
                action =
                    action.take().or_else(|| item(app, ui, true, K::Clear, Action::ClearRecent));
            }
        });
        // Booting the firmware needs a firmware image and a boot path the shim
        // does not offer: `mds_boot` always direct-boots a cart with FreeBIOS.
        unavailable(app, ui, K::BootFirmware, Unavailable::Bindings);
        ui.separator();

        ui.label(format!("{}: {}", app.i18n().t(K::DsSlot), app.cart_label()));
        action = action.take().or_else(|| item(app, ui, true, K::InsertCart, Action::InsertCart));
        action = action.take().or_else(|| item(app, ui, loaded, K::EjectCart, Action::EjectCart));
        ui.separator();

        // There is no GBA slot in the FFI: `mds_nds_new` takes one ROM.
        ui.label(format!("{}: {}", app.i18n().t(K::GbaSlot), app.i18n().t(K::None)));
        unavailable(app, ui, K::InsertRomCart, Unavailable::Bindings);
        unavailable(app, ui, K::InsertAddonCart, Unavailable::Bindings);
        unavailable(app, ui, K::EjectCart, Unavailable::Bindings);
        ui.separator();

        action = action
            .take()
            .or_else(|| item(app, ui, loaded, K::ImportSavefile, Action::ImportSavefile));
        ui.separator();

        ui.menu_button(app.i18n().s(K::SaveState), |ui| {
            for slot in 1..=STATE_SLOTS {
                action = action.take().or_else(|| {
                    entry(ui, loaded, &slot.to_string(), Action::SaveState(Some(slot)))
                });
            }
            ui.separator();
            action = action
                .take()
                .or_else(|| item(app, ui, loaded, K::FromFile, Action::SaveState(None)));
        });
        ui.menu_button(app.i18n().s(K::LoadState), |ui| {
            for slot in 1..=STATE_SLOTS {
                let exists = app.state_slot_exists(slot);
                action = action.take().or_else(|| {
                    entry(ui, loaded && exists, &slot.to_string(), Action::LoadState(Some(slot)))
                });
            }
            ui.separator();
            action = action
                .take()
                .or_else(|| item(app, ui, loaded, K::FromFile, Action::LoadState(None)));
        });
        action = action.take().or_else(|| {
            item(app, ui, app.can_undo_state_load(), K::UndoStateLoad, Action::UndoStateLoad)
        });
        ui.separator();

        action =
            action.take().or_else(|| item(app, ui, true, K::OpenDirectory, Action::OpenDirectory));
        ui.separator();

        action = action.take().or_else(|| item(app, ui, true, K::Quit, Action::Quit));
    });
    action
}

fn system_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button(app.i18n().s(K::SystemLabel), |ui| {
        let loaded = app.is_loaded();

        // Pause is a checkbox in melonDS, and reads as one here too.
        let mut paused = app.is_paused();
        let pause_label = app.i18n().s(K::Pause);
        if ui.add_enabled(loaded, egui::Checkbox::new(&mut paused, pause_label)).clicked() {
            ui.close();
            action = Some(Action::TogglePause);
        }
        action = action.take().or_else(|| item(app, ui, loaded, K::Reset, Action::Reset));
        action = action.take().or_else(|| item(app, ui, loaded, K::Stop, Action::Stop));
        action = action.take().or_else(|| item(app, ui, loaded, K::FrameStep, Action::FrameStep));
        ui.separator();

        action = action
            .take()
            .or_else(|| item(app, ui, loaded, K::PowerManagement, Action::TogglePane(Pane::Power)));
        action = action
            .take()
            .or_else(|| item(app, ui, loaded, K::DateAndTime, Action::TogglePane(Pane::DateTime)));
        ui.separator();

        // melonDS's AR engine, running the codes from the ARM7's VBlank
        // handler exactly as the hardware does.
        let mut cheats_on = app.cheats_enabled;
        let cheats_label = app.i18n().s(K::EnableCheats);
        if ui.checkbox(&mut cheats_on, cheats_label).clicked() {
            app.cheats_enabled = cheats_on;
        }
        action = action
            .take()
            .or_else(|| item(app, ui, true, K::SetupCheats, Action::TogglePane(Pane::Cheats)));
        ui.separator();

        action = action
            .take()
            .or_else(|| item(app, ui, loaded, K::RomInfo, Action::TogglePane(Pane::RomInfo)));
        action = action
            .take()
            .or_else(|| item(app, ui, loaded, K::RamSearch, Action::TogglePane(Pane::RamSearch)));
        // No DSi mode in this build at all.
        unavailable(app, ui, K::ManageDsiTitles, Unavailable::Bindings);
        ui.separator();

        ui.menu_button(app.i18n().s(K::Multiplayer), |ui| {
            let label = if app.has_guest() { K::CloseInstance } else { K::LaunchInstance };
            action = action.take().or_else(|| item(app, ui, loaded, label, Action::LaunchInstance));
            action = action.take().or_else(|| {
                item(app, ui, true, K::WirelessStatus, Action::TogglePane(Pane::Wireless))
            });
            ui.separator();
            ui.label(app.i18n().t(K::LanRoom));
            ui.monospace(&app.lan_room);
            ui.label(format!("{}: {}", app.i18n().t(K::HostBind), app.lan_bind_address));
            ui.label(format!("{}: {}", app.i18n().t(K::GuestIp), app.lan_guest_address));
            ui.small(&app.lan_status);
            // The one number that says whether a link is working; see
            // `crate::lan`.
            if let Some(stats) = app.lan_stats()
                && let Some(success) = stats.round_success()
            {
                ui.small(format!(
                    "{}: {:.0}%   {}: {:.0} ms   {}: {:.0} fps",
                    app.i18n().t(K::RoundsCompleted),
                    success * 100.0,
                    app.i18n().t(K::RoundTrip),
                    stats.rtt_ms,
                    app.i18n().t(K::SustainableFps),
                    stats.sustainable_fps,
                ));
            }
            ui.separator();
            action = action
                .take()
                .or_else(|| item(app, ui, loaded, K::HostLanGame, Action::HostLanGame));
            action = action
                .take()
                .or_else(|| item(app, ui, loaded, K::GuestLanGame, Action::GuestLanGame));
        });

        // Kept as a menu of its own beside Multiplayer rather than inside it:
        // it is not another way to play the same wireless game, it is a
        // different arrangement of the machines entirely.
        ui.menu_button(app.i18n().s(K::RemoteDesktop), |ui| {
            ui.small(app.i18n().t(K::RemoteDesktopExplained));
            ui.separator();
            let running = app.remote_running();
            action = action.take().or_else(|| {
                item(app, ui, loaded && !running, K::HostRemoteDesktop, Action::HostRemoteDesktop)
            });
            action = action.take().or_else(|| {
                item(app, ui, !running, K::JoinRemoteDesktop, Action::JoinRemoteDesktop)
            });
            action = action.take().or_else(|| {
                item(app, ui, running, K::StopRemoteDesktop, Action::StopRemoteDesktop)
            });
            if let Some(stats) = app.remote_stats {
                ui.separator();
                ui.small(format!(
                    "{}: {:.0} ms   {}: {:.0} fps, {:.2} Mbit/s",
                    app.i18n().t(K::InputLatency),
                    stats.rtt_ms,
                    app.i18n().t(K::Video),
                    stats.video_fps,
                    stats.video_megabits_per_second() + stats.audio_megabits_per_second(),
                ));
            }
        });
    });
    action
}

fn view_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button(app.i18n().s(K::ViewLabel), |ui| {
        ui.menu_button(app.i18n().s(K::ScreenSize), |ui| {
            for scale in 1..=4 {
                action = action.take().or_else(|| {
                    entry(ui, true, &format!("{scale}x"), Action::ScreenSize(scale as f32))
                });
            }
        });

        // The labels are read out of the map first: `app.i18n` borrows `app`,
        // and `app.view` below needs it mutably.
        let words = ViewWords::of(app);
        let view = &mut app.view;
        ui.menu_button(&words.rotation, |ui| {
            for rotation in Rotation::ALL {
                ui.radio_value(&mut view.rotation, rotation, format!("{}°", rotation.degrees()));
            }
        });
        ui.menu_button(&words.gap, |ui| {
            for gap in SCREEN_GAPS {
                ui.radio_value(&mut view.gap, gap, format!("{gap} px"));
            }
        });
        ui.menu_button(&words.layout, |ui| {
            for layout in ScreenLayout::ALL {
                ui.radio_value(&mut view.layout, layout, layout.label());
            }
            ui.separator();
            ui.checkbox(&mut view.swap, &words.swap);
        });
        ui.menu_button(&words.sizing, |ui| {
            for sizing in ScreenSizing::ALL {
                ui.radio_value(&mut view.sizing, sizing, sizing.label());
            }
            ui.separator();
            ui.checkbox(&mut view.integer_scaling, &words.integer_scaling);
        });
        ui.menu_button(&words.aspect, |ui| {
            // Per screen, and labelled per screen, exactly as melonDS lists it.
            for aspect in AspectRatio::ALL {
                ui.radio_value(
                    &mut view.aspect_top,
                    aspect,
                    format!("{} {}", words.top, aspect.label()),
                );
            }
            ui.separator();
            for aspect in AspectRatio::ALL {
                ui.radio_value(
                    &mut view.aspect_bottom,
                    aspect,
                    format!("{} {}", words.bottom, aspect.label()),
                );
            }
        });
        ui.separator();

        action = action.take().or_else(|| entry(ui, true, &words.new_window, Action::NewWindow));
        ui.separator();

        let view = &mut app.view;
        ui.checkbox(&mut view.filtering, &words.filtering);
        ui.checkbox(&mut view.show_osd, &words.show_osd);
    });
    action
}

fn config_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button(app.i18n().s(K::ConfigLabel), |ui| {
        for (key, pane) in
            [(K::EmuSettings, Pane::EmuSettings), (K::Preferences, Pane::Preferences)]
        {
            action = action.take().or_else(|| item(app, ui, true, key, Action::TogglePane(pane)));
        }
        ui.separator();

        action = action
            .take()
            .or_else(|| item(app, ui, true, K::InputAndHotkeys, Action::TogglePane(Pane::Input)));
        action = action.take().or_else(|| {
            item(app, ui, true, K::VideoSettings, Action::TogglePane(Pane::VideoSettings))
        });
        // No camera in the FFI.
        unavailable(app, ui, K::CameraSettings, Unavailable::Bindings);
        action = action.take().or_else(|| {
            item(app, ui, true, K::AudioSettings, Action::TogglePane(Pane::AudioSettings))
        });
        action = action.take().or_else(|| {
            item(app, ui, true, K::MultiplayerSettings, Action::TogglePane(Pane::Wireless))
        });
        action = action
            .take()
            .or_else(|| item(app, ui, true, K::WifiSettings, Action::TogglePane(Pane::Wireless)));
        // The firmware is generated by the shim; its contents are not settable.
        unavailable(app, ui, K::FirmwareSettings, Unavailable::Bindings);
        action = action.take().or_else(|| {
            item(app, ui, true, K::InterfaceSettings, Action::TogglePane(Pane::Interface))
        });
        action = action
            .take()
            .or_else(|| item(app, ui, true, K::PathSettings, Action::TogglePane(Pane::Paths)));
        ui.separator();

        let (limit, sync) = (app.i18n().s(K::LimitFramerate), app.i18n().s(K::AudioSync));
        ui.checkbox(&mut app.limit_framerate, limit);
        let has_audio = app.has_audio();
        ui.add_enabled(has_audio, egui::Checkbox::new(&mut app.audio_sync, sync))
            .on_disabled_hover_text("No audio output device.");
    });
    action
}

fn help_menu(app: &MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button(app.i18n().s(K::HelpLabel), |ui| {
        action = item(app, ui, true, K::About, Action::TogglePane(Pane::About));
    });
    action
}

/// The View menu's labels, taken out of the translation map before `app.view`
/// is borrowed mutably.
///
/// The borrow checker is the whole reason this exists: the menu's radio buttons
/// need `&mut app.view` for the length of the closure, and every label needs
/// `&app.i18n`. Copying eleven short strings once per menu open is cheaper than
/// the alternative and much clearer than interleaving the two.
struct ViewWords {
    rotation: String,
    gap: String,
    layout: String,
    swap: String,
    sizing: String,
    integer_scaling: String,
    aspect: String,
    top: String,
    bottom: String,
    new_window: String,
    filtering: String,
    show_osd: String,
}

impl ViewWords {
    fn of(app: &MelonEgui) -> Self {
        Self {
            rotation: app.i18n().s(K::ScreenRotation),
            gap: app.i18n().s(K::ScreenGap),
            layout: app.i18n().s(K::ScreenLayout),
            swap: app.i18n().s(K::SwapScreens),
            sizing: app.i18n().s(K::ScreenSizing),
            integer_scaling: app.i18n().s(K::IntegerScaling),
            aspect: app.i18n().s(K::AspectRatio),
            top: app.i18n().s(K::TopScreen),
            bottom: app.i18n().s(K::BottomScreen),
            new_window: app.i18n().s(K::NewWindow),
            filtering: app.i18n().s(K::ScreenFiltering),
            show_osd: app.i18n().s(K::ShowOsd),
        }
    }
}
